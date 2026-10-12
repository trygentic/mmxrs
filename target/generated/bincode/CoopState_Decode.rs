impl < __Context > :: bincode :: Decode < __Context > for CoopState
{
    fn decode < __D : :: bincode :: de :: Decoder < Context = __Context > >
    (decoder : & mut __D) ->core :: result :: Result < Self, :: bincode ::
    error :: DecodeError >
    {
        core :: result :: Result ::
        Ok(Self
        {
            enabled : :: bincode :: Decode :: decode(decoder) ?, p2 : ::
            bincode :: Decode :: decode(decoder) ?, p2_hidden : :: bincode ::
            Decode :: decode(decoder) ?, p2_oam_base : :: bincode :: Decode ::
            decode(decoder) ?, p2_scratch : :: bincode :: Decode ::
            decode(decoder) ?, p2_oam_count : :: bincode :: Decode ::
            decode(decoder) ?, dbg_regs : :: bincode :: Decode ::
            decode(decoder) ?, zero_palette : :: bincode :: Decode ::
            decode(decoder) ?, down_frames : :: bincode :: Decode ::
            decode(decoder) ?, contact_hits : :: bincode :: Decode ::
            decode(decoder) ?, in_contact : :: bincode :: Decode ::
            decode(decoder) ?, p2_on_obj : :: bincode :: Decode ::
            decode(decoder) ?, p2_items : :: bincode :: Decode ::
            decode(decoder) ?, p2_shots : :: bincode :: Decode ::
            decode(decoder) ?, p2_fx : :: bincode :: Decode :: decode(decoder)
            ?, zero_frame : :: bincode :: Decode :: decode(decoder) ?, saber :
            :: bincode :: Decode :: decode(decoder) ?, wave_frames : ::
            bincode :: Decode :: decode(decoder) ?, p1 : :: bincode :: Decode
            :: decode(decoder) ?, revive : :: bincode :: Decode ::
            decode(decoder) ?, p2_away : :: bincode :: Decode ::
            decode(decoder) ?, p2_warp : :: bincode :: Decode ::
            decode(decoder) ?, sfx : :: bincode :: Decode :: decode(decoder)
            ?,
        })
    }
} impl < '__de, __Context > :: bincode :: BorrowDecode < '__de, __Context >
for CoopState
{
    fn borrow_decode < __D : :: bincode :: de :: BorrowDecoder < '__de,
    Context = __Context > > (decoder : & mut __D) ->core :: result :: Result <
    Self, :: bincode :: error :: DecodeError >
    {
        core :: result :: Result ::
        Ok(Self
        {
            enabled : :: bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, p2 : :: bincode :: BorrowDecode ::< '_,
            __Context >:: borrow_decode(decoder) ?, p2_hidden : :: bincode ::
            BorrowDecode ::< '_, __Context >:: borrow_decode(decoder) ?,
            p2_oam_base : :: bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, p2_scratch : :: bincode :: BorrowDecode
            ::< '_, __Context >:: borrow_decode(decoder) ?, p2_oam_count : ::
            bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, dbg_regs : :: bincode :: BorrowDecode
            ::< '_, __Context >:: borrow_decode(decoder) ?, zero_palette : ::
            bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, down_frames : :: bincode :: BorrowDecode
            ::< '_, __Context >:: borrow_decode(decoder) ?, contact_hits : ::
            bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, in_contact : :: bincode :: BorrowDecode
            ::< '_, __Context >:: borrow_decode(decoder) ?, p2_on_obj : ::
            bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, p2_items : :: bincode :: BorrowDecode
            ::< '_, __Context >:: borrow_decode(decoder) ?, p2_shots : ::
            bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, p2_fx : :: bincode :: BorrowDecode ::<
            '_, __Context >:: borrow_decode(decoder) ?, zero_frame : ::
            bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, saber : :: bincode :: BorrowDecode ::<
            '_, __Context >:: borrow_decode(decoder) ?, wave_frames : ::
            bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, p1 : :: bincode :: BorrowDecode ::< '_,
            __Context >:: borrow_decode(decoder) ?, revive : :: bincode ::
            BorrowDecode ::< '_, __Context >:: borrow_decode(decoder) ?,
            p2_away : :: bincode :: BorrowDecode ::< '_, __Context >::
            borrow_decode(decoder) ?, p2_warp : :: bincode :: BorrowDecode ::<
            '_, __Context >:: borrow_decode(decoder) ?, sfx : :: bincode ::
            BorrowDecode ::< '_, __Context >:: borrow_decode(decoder) ?,
        })
    }
}