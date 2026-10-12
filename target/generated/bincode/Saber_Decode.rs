impl < __Context > :: bincode :: Decode < __Context > for Saber
{
    fn decode < __D : :: bincode :: de :: Decoder < Context = __Context > >
    (decoder : & mut __D) ->core :: result :: Result < Self, :: bincode ::
    error :: DecodeError >
    {
        core :: result :: Result ::
        Ok(Self
        {
            t : :: bincode :: Decode :: decode(decoder) ?, air : :: bincode ::
            Decode :: decode(decoder) ?, hit : :: bincode :: Decode ::
            decode(decoder) ?, btn : :: bincode :: Decode :: decode(decoder)
            ?, slot : :: bincode :: Decode :: decode(decoder) ?,
        })
    }
} impl < '__de, __Context > :: bincode :: BorrowDecode < '__de, __Context >
for Saber
{
    fn borrow_decode < __D : :: bincode :: de :: BorrowDecoder < '__de,
    Context = __Context > > (decoder : & mut __D) ->core :: result :: Result <
    Self, :: bincode :: error :: DecodeError >
    {
        core :: result :: Result ::
        Ok(Self
        {
            t : :: bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, air : :: bincode :: BorrowDecode ::< '_,
            __Context >:: borrow_decode(decoder) ?, hit : :: bincode ::
            BorrowDecode ::< '_, __Context >:: borrow_decode(decoder) ?, btn :
            :: bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, slot : :: bincode :: BorrowDecode ::<
            '_, __Context >:: borrow_decode(decoder) ?,
        })
    }
}