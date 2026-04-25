use super::shift_lrc_timestamps;

#[test]
fn shifts_line_level_timestamps() {
    let src = "[00:01.200]A\n[00:02.000]B\n";
    let (out, count) = shift_lrc_timestamps(src, 500);
    assert_eq!(count, 2);
    assert_eq!(out, "[00:01.700]A\n[00:02.500]B\n");
}

#[test]
fn shifts_enhanced_and_char_level_timestamps() {
    let src = "[00:10.000]<00:10.100>あ<00:10.250>い\n[00:11.000][00:11.100]A\n";
    let (out, count) = shift_lrc_timestamps(src, -150);
    assert_eq!(count, 5);
    assert_eq!(
        out,
        "[00:09.850]<00:09.950>あ<00:10.100>い\n[00:10.850][00:10.950]A\n"
    );
}

#[test]
fn keeps_metadata_and_clamps_to_zero() {
    let src = "[ar:Artist]\n[00:00.050]Intro\n";
    let (out, count) = shift_lrc_timestamps(src, -200);
    assert_eq!(count, 1);
    assert_eq!(out, "[ar:Artist]\n[00:00.000]Intro\n");
}
