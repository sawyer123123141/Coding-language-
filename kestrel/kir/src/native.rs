/// The symbol a `native` function imports: `kestrel_` and its qualified name
/// with each `.` turned into `_`. `draw.circle` is `kestrel_draw_circle`; a
/// native named `twice` in the entry file is `kestrel_twice`.
pub fn native_symbol(qualified: &str) -> String {
    format!("kestrel_{}", qualified.replace('.', "_"))
}

#[cfg(test)]
mod tests {
    use super::native_symbol;

    #[test]
    fn symbols_join_the_module_and_the_name() {
        assert_eq!(native_symbol("draw.circle"), "kestrel_draw_circle");
        assert_eq!(native_symbol("twice"), "kestrel_twice");
        assert_eq!(native_symbol("window.screenshot"), "kestrel_window_screenshot");
    }
}
