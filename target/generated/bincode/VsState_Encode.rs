impl :: bincode :: Encode for VsState
{
    fn encode < __E : :: bincode :: enc :: Encoder >
    (& self, encoder : & mut __E) ->core :: result :: Result < (), :: bincode
    :: error :: EncodeError >
    {
        :: bincode :: Encode :: encode(&self.winner, encoder) ?; :: bincode ::
        Encode :: encode(&self.round_over, encoder) ?; :: bincode :: Encode ::
        encode(&self.scores, encoder) ?; :: bincode :: Encode ::
        encode(&self.round, encoder) ?; :: bincode :: Encode ::
        encode(&self.players, encoder) ?; :: bincode :: Encode ::
        encode(&self.placed, encoder) ?; :: bincode :: Encode ::
        encode(&self.arena_left, encoder) ?; :: bincode :: Encode ::
        encode(&self.origin, encoder) ?; :: bincode :: Encode ::
        encode(&self.eliminated, encoder) ?; :: bincode :: Encode ::
        encode(&self.cooldown, encoder) ?; core :: result :: Result :: Ok(())
    }
}