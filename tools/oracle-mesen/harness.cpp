// mesen-oracle: play a BizHawk bk2 input log on Mesen2's SNES core, headless,
// from power-on, and write one trace line per video frame in the same format
// as `cargo xtask movie --trace` (see README.md).
//
//   mesen-oracle ROM INPUTLOG FRAMES TRACE [--dump F|A-B/S,..] [--dump-dir DIR] [--ram zero|ones|random]
//                [--spc-adjust N] (SPC sample rate 32000+N Hz, default 40 like Mesen's UI)
//                [--bsnes-apu 1] (SPC clock ratio of bsnes v0.87 instead, see Makefile)
//                [--cpu-trace A-B PATH] (every instruction of frames A..B: PC scanline H)
//                [--spc-trace 1] (with --cpu-trace: SPC700 rows too, "S PC cycle scanline H")
//
// INPUTLOG is the bk2's "Input Log.txt" (unzip -p movie.bk2 'Input Log.txt').
// Line k (0-based) of the log is the pad for video frame k+1.
//
// Mesen calls the input providers once at power-on (poll 0) and then at the
// start of every vblank (Snes PPU -> SnesConsole::ProcessEndOfFrame ->
// UpdateInputState). Poll p therefore ends video frame p, and it sets the
// pad for frame p+1, which the auto-joypad read of that same vblank latches.
#include "pch.h"
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <unistd.h>
#include "Shared/Emulator.h"
#include "Shared/EmuSettings.h"
#include "Shared/SettingTypes.h"
#include "Shared/BaseControlDevice.h"
#include "Shared/BaseControlManager.h"
#include "Shared/NotificationManager.h"
#include "Shared/Interfaces/IInputProvider.h"
#include "Shared/Interfaces/INotificationListener.h"
#include "Shared/MemoryType.h"
#include "SNES/SnesConsole.h"
#include "SNES/SnesCpu.h"
#include "SNES/SnesPpu.h"
#include "SNES/SnesMemoryManager.h"
#include "SNES/Input/SnesController.h"
#include "Utilities/VirtualFile.h"
#include "Utilities/FolderUtilities.h"
#include "Debugger/Debugger.h"
#include "Debugger/ITraceLogger.h"
#include "Debugger/TraceLogFileSaver.h"

extern double mesenOracleSpcRate; // patched Spc.cpp (Makefile)
static vector<uint16_t> g_pads;    // our pad word: B Y Sl St U D L R A X L R 0 0 0 0 (bit 15 = B)
static uint32_t g_frames = 0;
static FILE* g_trace = nullptr;
static vector<uint32_t> g_dump;
static string g_dumpDir = ".";
static Emulator* g_emu = nullptr;
static uint32_t g_cpuTraceFrom = 0, g_cpuTraceTo = 0; // frames A..B (1-based, as in the trace)
static string g_cpuTracePath;
static bool g_spcTrace = false;

static uint64_t fnv1a(const uint8_t* p, size_t n)
{
	uint64_t h = 0xcbf29ce484222325ULL;
	for(size_t i = 0; i < n; i++) {
		h ^= p[i];
		h *= 0x100000001b3ULL;
	}
	return h;
}

// bk2 P1 columns: U D L R s S Y B X A l r
static uint16_t parseLine(const string& line)
{
	// "|..|UDLRsSYBXAlr|............|"
	size_t a = line.find('|', 1);
	if(a == string::npos || line.size() < a + 13) {
		return 0;
	}
	static const uint16_t bits[12] = {
		0x0800, 0x0400, 0x0200, 0x0100, // U D L R
		0x2000, 0x1000, 0x4000, 0x8000, // Select Start Y B
		0x0040, 0x0080, 0x0020, 0x0010  // X A L R
	};
	uint16_t v = 0;
	for(int i = 0; i < 12; i++) {
		if(line[a + 1 + i] != '.') {
			v |= bits[i];
		}
	}
	return v;
}

static void setButtons(BaseControlDevice* dev, uint16_t pad)
{
	struct M { uint16_t bit; SnesController::Buttons b; };
	static const M map[12] = {
		{ 0x8000, SnesController::B }, { 0x4000, SnesController::Y },
		{ 0x2000, SnesController::Select }, { 0x1000, SnesController::Start },
		{ 0x0800, SnesController::Up }, { 0x0400, SnesController::Down },
		{ 0x0200, SnesController::Left }, { 0x0100, SnesController::Right },
		{ 0x0080, SnesController::A }, { 0x0040, SnesController::X },
		{ 0x0020, SnesController::L }, { 0x0010, SnesController::R },
	};
	for(const M& m : map) {
		if(pad & m.bit) {
			dev->SetBit(m.b);
		}
	}
}

static void startCpuTrace(Debugger* dbg)
{
	if(!dbg) {
		fprintf(stderr, "mesen-oracle: no debugger, no --cpu-trace\n");
		return;
	}
	TraceLoggerOptions o = {};
	o.Enabled = true;
	strcpy(o.Format, "[PC] [Scanline] [HClock]");
	dbg->GetTraceLogger(CpuType::Snes)->SetOptions(o);
	if(g_spcTrace) {
		// SPC700 rows in the same file, told apart by the leading "S"
		TraceLoggerOptions so = {};
		so.Enabled = true;
		strcpy(so.Format, "S [PC] [CycleCount] [Scanline] [HClock]");
		dbg->GetTraceLogger(CpuType::Spc)->SetOptions(so);
	}
	dbg->GetTraceLogFileSaver()->StartLogging(g_cpuTracePath);
}

class Oracle : public IInputProvider, public INotificationListener
{
public:
	bool SetInput(BaseControlDevice* device) override
	{
		if(device->GetPort() != 0) {
			return true; // P2: nothing pressed
		}
		SnesConsole* console = (SnesConsole*)g_emu->GetConsole().get();
		uint32_t p = console->GetControlManager()->GetPollCounter();
		if(p >= 1) {
			logFrame(p, console);
		}
		if(p >= g_frames) {
			fflush(g_trace);
			fclose(g_trace);
			_exit(0);
		}
		if(!g_cpuTracePath.empty()) {
			// Frame p+1 runs next: log its instructions when it is in A..B.
			Debugger* dbg = g_emu->InternalGetDebugger();
			if(dbg && p + 1 == g_cpuTraceFrom) {
				startCpuTrace(dbg);
			} else if(dbg && p == g_cpuTraceTo) {
				dbg->GetTraceLogFileSaver()->StopLogging();
			}
		}
		uint16_t pad = p < g_pads.size() ? g_pads[p] : 0;
		setButtons(device, pad);
		return true;
	}

	void logFrame(uint32_t f, SnesConsole* console)
	{
		ConsoleMemoryInfo m = g_emu->GetMemory(MemoryType::SnesWorkRam);
		const uint8_t* w = (const uint8_t*)m.Memory;
		auto w16 = [&](uint32_t a) { return (uint32_t)w[a] | ((uint32_t)w[a + 1] << 8); };
		SnesCpuState& cpu = console->GetCpu()->GetState();
		uint64_t mclk = console->GetMemoryManager()->GetMasterClock();
		// f hash main nmi rng xx xy pad mode level lives hp stk | extra (Mesen only): master clock, PC
		fprintf(g_trace, "%u %016llx %02x %02x %04x %04x %04x %04x %02x%02x%02x%02x %02x %02x %02x %02x | mclk %llu pc %02x%04x\n", f,
			(unsigned long long)fnv1a(w, 0x20000), w[0x0B9B], w[0x0B9E], w16(0x0BA6), w16(0x0BAD), w16(0x0BB0), w16(0x00A7),
			w[0xD1], w[0xD2], w[0xD3], w[0xD4], w[0x1F7A], w[0x1F80], w[0x0BCF] & 0x7F, w[0x02FD],
			(unsigned long long)mclk, cpu.K, cpu.PC);
		for(uint32_t d : g_dump) {
			if(d == f) {
				string path = g_dumpDir + "/mesen-wram-" + std::to_string(f) + ".bin";
				std::ofstream(path, std::ios::binary).write((const char*)w, 0x20000);
			}
		}
	}

	void ProcessNotification(ConsoleNotificationType type, void* parameter) override
	{
		if(type == ConsoleNotificationType::GameLoaded) {
			g_emu->RegisterInputProvider(this);
			if(!g_cpuTracePath.empty() && g_cpuTraceFrom <= 1) {
				startCpuTrace(g_emu->InternalGetDebugger());
			}
		}
	}
};

int main(int argc, char** argv)
{
	if(argc < 5) {
		fprintf(stderr, "usage: mesen-oracle ROM INPUTLOG FRAMES TRACE [--dump F,..] [--dump-dir DIR] [--ram zero|ones|random] [--spc-adjust N] [--cpu-trace A-B PATH]\n");
		return 2;
	}
	string rom = argv[1];
	g_frames = (uint32_t)atoi(argv[3]);
	RamState ram = RamState::AllZeros;
	int32_t spcAdjust = 40; // Mesen UI default: SPC at 32040 Hz (bsnes: 24607104/768 = 32040.5 Hz)
	for(int i = 5; i + 1 < argc; i += 2) {
		string k = argv[i], v = argv[i + 1];
		if(k == "--dump") {
			size_t s = 0;
			while(s < v.size()) {
				size_t e = v.find(',', s);
				// F, or A-B/S (every S frames from A to B)
				string item = v.substr(s, e == string::npos ? string::npos : e - s);
				uint32_t a = 0, b = 0, st = 1;
				if(sscanf(item.c_str(), "%u-%u/%u", &a, &b, &st) >= 2) {
					for(uint32_t f = a; f <= b; f += (st ? st : 1)) {
						g_dump.push_back(f);
					}
				} else {
					g_dump.push_back((uint32_t)atoi(item.c_str()));
				}
				s = e == string::npos ? v.size() : e + 1;
			}
		} else if(k == "--dump-dir") {
			g_dumpDir = v;
		} else if(k == "--cpu-trace" && i + 2 < argc) {
			sscanf(v.c_str(), "%u-%u", &g_cpuTraceFrom, &g_cpuTraceTo);
			g_cpuTracePath = argv[i + 2];
			i++;
		} else if(k == "--spc-trace") {
			g_spcTrace = v == "1";
		} else if(k == "--bsnes-apu") {
			// SPC ticks (2.048 MHz units) per second at Mesen's master clock rate, for
			// bsnes's ratio 24607104 / 21477272 (= 32040.5 Hz * 64 at 21477272 Hz)
			if(v == "1") {
				mesenOracleSpcRate = 24607104.0 / 12.0 * 21477270.0 / 21477272.0;
			}
		} else if(k == "--spc-adjust") {
			spcAdjust = atoi(v.c_str());
		} else if(k == "--ram") {
			ram = v == "ones" ? RamState::AllOnes : (v == "random" ? RamState::Random : RamState::AllZeros);
		}
	}
	{
		std::ifstream in(argv[2]);
		string line;
		while(std::getline(in, line)) {
			if(line.size() > 1 && line[0] == '|') {
				g_pads.push_back(parseLine(line));
			}
		}
	}
	g_trace = fopen(argv[4], "w");
	if(!g_trace) {
		perror(argv[4]);
		return 1;
	}
	fprintf(stderr, "mesen-oracle: %zu input lines, %u frames\n", g_pads.size(), g_frames);

	// Mesen writes battery/recent-game files under its home folder: keep them out of ~.
	FolderUtilities::SetHomeFolder(g_dumpDir + "/mesen-home");
	Emulator* emu = new Emulator();
	g_emu = emu;
	emu->Initialize(false);
	EmuSettings* s = emu->GetSettings();
	SnesConfig cfg = s->GetSnesConfig();
	cfg.Port1.Type = ControllerType::SnesController;
	cfg.Port2.Type = ControllerType::SnesController;
	cfg.Region = ConsoleRegion::Ntsc;
	cfg.RamPowerOnState = ram;
	cfg.EnableRandomPowerOnState = false;
	cfg.SpcClockSpeedAdjustment = spcAdjust;
	cfg.DisableFrameSkipping = true;
	s->SetSnesConfig(cfg);
	EmulationConfig ec = s->GetEmulationConfig();
	ec.EmulationSpeed = 0;
	ec.RunAheadFrames = 0;
	s->SetEmulationConfig(ec);

	if(!g_cpuTracePath.empty()) {
		// The trace logger needs the debugger, which LoadRom starts in console mode.
		s->SetFlag(EmulationFlags::ConsoleMode);
	}
	shared_ptr<Oracle> oracle(new Oracle());
	emu->GetNotificationManager()->RegisterNotificationListener(oracle);
	if(!emu->LoadRom(VirtualFile(rom), VirtualFile())) {
		fprintf(stderr, "could not load %s\n", rom.c_str());
		return 1;
	}
	for(;;) {
		sleep(1);
	}
}
