use intervals::merge;

#[test]
fn merges_overlapping() {
    assert_eq!(merge(vec![(1, 3), (2, 6), (8, 10), (15, 18)]), vec![(1, 6), (8, 10), (15, 18)]);
}
#[test]
fn merges_touching() {
    assert_eq!(merge(vec![(1, 4), (4, 5)]), vec![(1, 5)]);
}
#[test]
fn keeps_longer_end() {
    assert_eq!(merge(vec![(1, 10), (2, 3)]), vec![(1, 10)]);
}
#[test]
fn unsorted() {
    assert_eq!(merge(vec![(5, 6), (1, 2)]), vec![(1, 2), (5, 6)]);
}
