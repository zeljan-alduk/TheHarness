pub fn sum_even_squares(xs: &[i64]) -> i64 {
    let mut total = 0;
    for x in xs {
        if x % 2 == 0 {
            total += x * x;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sums() {
        assert_eq!(sum_even_squares(&[1, 2, 3, 4]), 20);
        assert_eq!(sum_even_squares(&[]), 0);
        assert_eq!(sum_even_squares(&[-2, 3]), 4);
    }
}
