impl :: bincode :: Encode for Saber
{
    fn encode < __E : :: bincode :: enc :: Encoder >
    (& self, encoder : & mut __E) ->core :: result :: Result < (), :: bincode
    :: error :: EncodeError >
    {
        :: bincode :: Encode :: encode(&self.t, encoder) ?; :: bincode ::
        Encode :: encode(&self.air, encoder) ?; :: bincode :: Encode ::
        encode(&self.hit, encoder) ?; :: bincode :: Encode ::
        encode(&self.btn, encoder) ?; :: bincode :: Encode ::
        encode(&self.slot, encoder) ?; core :: result :: Result :: Ok(())
    }
}