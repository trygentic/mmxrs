impl < __Context > :: bincode :: Decode < __Context > for Revive
{
    fn decode < __D : :: bincode :: de :: Decoder < Context = __Context > >
    (decoder : & mut __D) ->core :: result :: Result < Self, :: bincode ::
    error :: DecodeError >
    {
        core :: result :: Result ::
        Ok(Self
        {
            spot : :: bincode :: Decode :: decode(decoder) ?, touch : ::
            bincode :: Decode :: decode(decoder) ?, half_hp : :: bincode ::
            Decode :: decode(decoder) ?,
        })
    }
} impl < '__de, __Context > :: bincode :: BorrowDecode < '__de, __Context >
for Revive
{
    fn borrow_decode < __D : :: bincode :: de :: BorrowDecoder < '__de,
    Context = __Context > > (decoder : & mut __D) ->core :: result :: Result <
    Self, :: bincode :: error :: DecodeError >
    {
        core :: result :: Result ::
        Ok(Self
        {
            spot : :: bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, touch : :: bincode :: BorrowDecode ::<
            '_, __Context >:: borrow_decode(decoder) ?, half_hp : :: bincode
            :: BorrowDecode ::< '_, __Context >:: borrow_decode(decoder) ?,
        })
    }
}