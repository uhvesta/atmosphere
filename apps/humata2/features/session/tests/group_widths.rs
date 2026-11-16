//! The width of each tab group: the group beside a Side-by-side document is
//! narrow, otherwise groups are equal.
use humata2_session::{GroupWidth, group_widths};

#[test]
fn the_group_beside_a_side_by_side_document_is_330_wide() {
    assert_eq!(
        group_widths(979., &[false, true]),
        [GroupWidth::Fixed(330.), GroupWidth::Flexible]
    );
    assert_eq!(
        group_widths(979., &[true, false]),
        [GroupWidth::Flexible, GroupWidth::Fixed(330.)]
    );
}

#[test]
fn groups_are_equal_unless_one_shows_a_side_by_side_document() {
    assert_eq!(
        group_widths(979., &[false, false]),
        [GroupWidth::Flexible, GroupWidth::Flexible]
    );
    assert_eq!(
        group_widths(979., &[true, true]),
        [GroupWidth::Flexible, GroupWidth::Flexible]
    );
    assert_eq!(group_widths(979., &[false]), [GroupWidth::Flexible]);
}

#[test]
fn a_narrow_row_gives_the_beside_group_half() {
    assert_eq!(
        group_widths(400., &[false, true]),
        [GroupWidth::Fixed(200.), GroupWidth::Flexible]
    );
}
