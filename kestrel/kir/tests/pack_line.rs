use kestrel_kir::{pack_line, unpack_line, NO_FILE};

#[test]
fn a_line_and_its_file_round_trip() {
    assert_eq!(unpack_line(pack_line(0, 24)), (24, 0));
    assert_eq!(unpack_line(pack_line(7, 24)), (24, 7));
    assert_eq!(unpack_line(pack_line(1022, 2_097_151)), (2_097_151, 1022));
    // One unnamed file with a plain line keeps the plain line.
    assert_eq!(pack_line(0, 24), 24);
}

#[test]
fn a_file_or_line_out_of_range_is_clamped() {
    assert_eq!(unpack_line(pack_line(1023, 5)), (5, NO_FILE));
    assert_eq!(unpack_line(pack_line(5000, 5)), (5, NO_FILE));
    assert_eq!(unpack_line(pack_line(usize::MAX, 5)), (5, NO_FILE));
    assert_eq!(unpack_line(pack_line(3, u32::MAX)), (2_097_151, 3));
    // Bit 31 stays clear, so the line is a valid positive i32 wherever it is read as one.
    assert!(pack_line(usize::MAX, u32::MAX) < 1 << 31);
}
