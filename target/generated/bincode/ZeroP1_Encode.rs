impl :: bincode :: Encode for ZeroP1
{
    fn encode < __E : :: bincode :: enc :: Encoder >
    (& self, encoder : & mut __E) ->core :: result :: Result < (), :: bincode
    :: error :: EncodeError >
    {
        :: bincode :: Encode :: encode(&self.on, encoder) ?; :: bincode ::
        Encode :: encode(&self.frame, encoder) ?; :: bincode :: Encode ::
        encode(&self.saber, encoder) ?; core :: result :: Result :: Ok(())
    }
}