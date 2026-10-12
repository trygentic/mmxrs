impl < __Context > :: bincode :: Decode < __Context > for ZeroP1
{
    fn decode < __D : :: bincode :: de :: Decoder < Context = __Context > >
    (decoder : & mut __D) ->core :: result :: Result < Self, :: bincode ::
    error :: DecodeError >
    {
        core :: result :: Result ::
        Ok(Self
        {
            on : :: bincode :: Decode :: decode(decoder) ?, frame : :: bincode
            :: Decode :: decode(decoder) ?, saber : :: bincode :: Decode ::
            decode(decoder) ?,
        })
    }
} impl < '__de, __Context > :: bincode :: BorrowDecode < '__de, __Context >
for ZeroP1
{
    fn borrow_decode < __D : :: bincode :: de :: BorrowDecoder < '__de,
    Context = __Context > > (decoder : & mut __D) ->core :: result :: Result <
    Self, :: bincode :: error :: DecodeError >
    {
        core :: result :: Result ::
        Ok(Self
        {
            on : :: bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, frame : :: bincode :: BorrowDecode ::<
            '_, __Context >:: borrow_decode(decoder) ?, saber : :: bincode ::
            BorrowDecode ::< '_, __Context >:: borrow_decode(decoder) ?,
        })
    }
}