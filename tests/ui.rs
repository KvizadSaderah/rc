use rust_commander::ui::fit_name;

#[test]
fn fit_name_multibyte_no_panic() {
    // Regression: byte-slicing a name with multibyte chars panicked at a
    // non-char-boundary ("end byte index N is not a char boundary").
    let name = "Загрузки-отчёт-файл.txt"; // Cyrillic: 2 bytes per char
    for w in 0..40usize {
        let out = fit_name(name, 0, w); // must not panic for any width
        assert!(out.chars().count() <= w.max(3));
    }
    // Emoji (4-byte) name truncated mid-name must stay valid UTF-8.
    let emoji = "📁folder-with-a-long-name";
    let out = fit_name(emoji, 0, 10);
    assert!(out.ends_with("..."));
    assert!(std::str::from_utf8(out.as_bytes()).is_ok());
}

#[test]
fn fit_name_fits_unchanged() {
    assert_eq!(fit_name("short.rs", 0, 20), "short.rs");
    assert_eq!(fit_name("файл.rs", 0, 20), "файл.rs");
}

#[test]
fn fit_name_truncates_with_ellipsis() {
    let out = fit_name("a-fairly-long-filename.rs", 0, 12);
    assert!(out.ends_with("..."));
    assert!(out.chars().count() <= 12);
}
