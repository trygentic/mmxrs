//! Character choices shared by the launcher, native frontend and game core.
//!
//! The ROM's player routines still provide movement and combat. A character's
//! art is selected separately, so an animation absent from a boss sprite set
//! can fall back to its idle frame without changing those routines.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Character {
    #[default]
    X,
    Zero,
    ChillPenguin,
    SparkMandrill,
    ArmoredArmadillo,
    LaunchOctopus,
    BoomerKuwanger,
    StingChameleon,
    StormEagle,
    FlameMammoth,
    Vile,
    Sigma,
    WolfSigma,
    Velguarder,
    Bospider,
    RangdaBangda,
    DRex,
    Bass,
    ProtoMan,
}

impl Character {
    pub const ALL: &'static [Self] = &[
        Self::X,
        Self::Zero,
        Self::ChillPenguin,
        Self::SparkMandrill,
        Self::ArmoredArmadillo,
        Self::LaunchOctopus,
        Self::BoomerKuwanger,
        Self::StingChameleon,
        Self::StormEagle,
        Self::FlameMammoth,
        Self::Vile,
        Self::Sigma,
        Self::WolfSigma,
        Self::Velguarder,
        Self::Bospider,
        Self::RangdaBangda,
        Self::DRex,
        Self::Bass,
        Self::ProtoMan,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Zero => "zero",
            Self::ChillPenguin => "chill-penguin",
            Self::SparkMandrill => "spark-mandrill",
            Self::ArmoredArmadillo => "armored-armadillo",
            Self::LaunchOctopus => "launch-octopus",
            Self::BoomerKuwanger => "boomer-kuwanger",
            Self::StingChameleon => "sting-chameleon",
            Self::StormEagle => "storm-eagle",
            Self::FlameMammoth => "flame-mammoth",
            Self::Vile => "vile",
            Self::Sigma => "sigma",
            Self::WolfSigma => "wolf-sigma",
            Self::Velguarder => "velguarder",
            Self::Bospider => "bospider",
            Self::RangdaBangda => "rangda-bangda",
            Self::DRex => "d-rex",
            Self::Bass => "bass",
            Self::ProtoMan => "proto-man",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::X => "X",
            Self::Zero => "Zero",
            Self::ChillPenguin => "Chill Penguin",
            Self::SparkMandrill => "Spark Mandrill",
            Self::ArmoredArmadillo => "Armored Armadillo",
            Self::LaunchOctopus => "Launch Octopus",
            Self::BoomerKuwanger => "Boomer Kuwanger",
            Self::StingChameleon => "Sting Chameleon",
            Self::StormEagle => "Storm Eagle",
            Self::FlameMammoth => "Flame Mammoth",
            Self::Vile => "Vile",
            Self::Sigma => "Sigma",
            Self::WolfSigma => "Wolf Sigma",
            Self::Velguarder => "Velguarder",
            Self::Bospider => "Bospider",
            Self::RangdaBangda => "Rangda Bangda",
            Self::DRex => "D-Rex",
            Self::Bass => "Bass",
            Self::ProtoMan => "Proto Man",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|c| c.id().eq_ignore_ascii_case(s.trim()))
    }

    /// Bosses have no animation for every X action. Use their idle pose in
    /// those cases; the game keeps X's physics and controls.
    pub const fn fallback_animation(self) -> &'static str {
        match self {
            Self::X | Self::Zero => "matching",
            _ => "idle",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Character;

    #[test]
    fn roster_ids_are_unique_and_round_trip() {
        for &character in Character::ALL {
            assert_eq!(Character::parse(character.id()), Some(character));
            assert_eq!(
                Character::ALL
                    .iter()
                    .filter(|&&c| c.id() == character.id())
                    .count(),
                1
            );
        }
    }
}
