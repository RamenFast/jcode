/// Safely truncate a string at a character boundary.
pub(super) fn safe_truncate(value: &str, max_chars: usize) -> &str {
    if value.chars().count() <= max_chars {
        return value;
    }

    match value.char_indices().nth(max_chars) {
        Some((index, _)) => &value[..index],
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use super::safe_truncate;

    #[test]
    fn truncates_at_character_boundaries() {
        assert_eq!(safe_truncate("aé日z", 3), "aé日");
        assert_eq!(safe_truncate("short", 10), "short");
    }
}
