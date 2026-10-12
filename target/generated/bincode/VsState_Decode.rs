impl < __Context > :: bincode :: Decode < __Context > for VsState
{
    fn decode < __D : :: bincode :: de :: Decoder < Context = __Context > >
    (decoder : & mut __D) ->core :: result :: Result < Self, :: bincode ::
    error :: DecodeError >
    {
        core :: result :: Result ::
        Ok(Self
        {
            winner : :: bincode :: Decode :: decode(decoder) ?, round_over :
            :: bincode :: Decode :: decode(decoder) ?, scores : :: bincode ::
            Decode :: decode(decoder) ?, round : :: bincode :: Decode ::
            decode(decoder) ?, players : :: bincode :: Decode ::
            decode(decoder) ?, placed : :: bincode :: Decode ::
            decode(decoder) ?, arena_left : :: bincode :: Decode ::
            decode(decoder) ?, origin : :: bincode :: Decode ::
            decode(decoder) ?, eliminated : :: bincode :: Decode ::
            decode(decoder) ?, cooldown : :: bincode :: Decode ::
            decode(decoder) ?,
        })
    }
} impl < '__de, __Context > :: bincode :: BorrowDecode < '__de, __Context >
for VsState
{
    fn borrow_decode < __D : :: bincode :: de :: BorrowDecoder < '__de,
    Context = __Context > > (decoder : & mut __D) ->core :: result :: Result <
    Self, :: bincode :: error :: DecodeError >
    {
        core :: result :: Result ::
        Ok(Self
        {
            winner : :: bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, round_over : :: bincode :: BorrowDecode
            ::< '_, __Context >:: borrow_decode(decoder) ?, scores : ::
            bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, round : :: bincode :: BorrowDecode ::<
            '_, __Context >:: borrow_decode(decoder) ?, players : :: bincode
            :: BorrowDecode ::< '_, __Context >:: borrow_decode(decoder) ?,
            placed : :: bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, arena_left : :: bincode :: BorrowDecode
            ::< '_, __Context >:: borrow_decode(decoder) ?, origin : ::
            bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, eliminated : :: bincode :: BorrowDecode
            ::< '_, __Context >:: borrow_decode(decoder) ?, cooldown : ::
            bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?,
        })
    }
}