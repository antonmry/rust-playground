pub fn handle_wrap(current: u64, initial: u64, max: u64) -> u64 {
    if current >= initial {
        current - initial
    } else {
        (max - initial) + current
    }
}

#[cfg(test)]
mod tests {
    use super::handle_wrap;

    #[test]
    fn wrap_forward() {
        assert_eq!(handle_wrap(120, 100, 200), 20);
    }

    #[test]
    fn wrap_across() {
        assert_eq!(handle_wrap(10, 190, 200), 20);
    }
}
