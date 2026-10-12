impl :: bincode :: Encode for CoopState
{
    fn encode < __E : :: bincode :: enc :: Encoder >
    (& self, encoder : & mut __E) ->core :: result :: Result < (), :: bincode
    :: error :: EncodeError >
    {
        :: bincode :: Encode :: encode(&self.enabled, encoder) ?; :: bincode
        :: Encode :: encode(&self.p2, encoder) ?; :: bincode :: Encode ::
        encode(&self.p2_hidden, encoder) ?; :: bincode :: Encode ::
        encode(&self.p2_oam_base, encoder) ?; :: bincode :: Encode ::
        encode(&self.p2_scratch, encoder) ?; :: bincode :: Encode ::
        encode(&self.p2_oam_count, encoder) ?; :: bincode :: Encode ::
        encode(&self.dbg_regs, encoder) ?; :: bincode :: Encode ::
        encode(&self.zero_palette, encoder) ?; :: bincode :: Encode ::
        encode(&self.down_frames, encoder) ?; :: bincode :: Encode ::
        encode(&self.contact_hits, encoder) ?; :: bincode :: Encode ::
        encode(&self.in_contact, encoder) ?; :: bincode :: Encode ::
        encode(&self.p2_on_obj, encoder) ?; :: bincode :: Encode ::
        encode(&self.p2_items, encoder) ?; :: bincode :: Encode ::
        encode(&self.p2_shots, encoder) ?; :: bincode :: Encode ::
        encode(&self.p2_fx, encoder) ?; :: bincode :: Encode ::
        encode(&self.zero_frame, encoder) ?; :: bincode :: Encode ::
        encode(&self.saber, encoder) ?; :: bincode :: Encode ::
        encode(&self.wave_frames, encoder) ?; :: bincode :: Encode ::
        encode(&self.p1, encoder) ?; :: bincode :: Encode ::
        encode(&self.revive, encoder) ?; :: bincode :: Encode ::
        encode(&self.p2_away, encoder) ?; :: bincode :: Encode ::
        encode(&self.p2_warp, encoder) ?; :: bincode :: Encode ::
        encode(&self.sfx, encoder) ?; core :: result :: Result :: Ok(())
    }
}