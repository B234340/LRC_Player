//! Windows: embeds the application icon and version information into the .exe (shown by
//! Explorer, the taskbar before the window exists, shortcuts and the file properties).
//! Uses `windres` from MinGW (`x86_64-w64-mingw32-windres` when cross-compiling, `windres`
//! natively, or `$WINDRES`); without it the build still succeeds, just without the icon.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/icon/lrc_player.ico");
    println!("cargo:rerun-if-env-changed=WINDRES");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "windows" {
        return;
    }
    if env != "gnu" {
        // MSVC would need rc.exe; not used for the release builds of this project.
        println!("cargo:warning=no icon resource for target env {env:?} (only the GNU toolchain is wired up)");
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ico = manifest.join("../../assets/icon/lrc_player.ico").canonicalize().expect("assets/icon/lrc_player.ico");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut nums: Vec<u16> = version.split(['.', '-']).take(3).map(|p| p.parse().unwrap_or(0)).collect();
    nums.resize(3, 0);
    let (a, b, c) = (nums[0], nums[1], nums[2]);
    let rc = format!(
        r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "LRC Player"
      VALUE "ProductName", "LRC Player"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "lrc_player_gui.exe"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#,
        ico = ico.display().to_string().replace('\\', "/"),
    );
    let rc_path = out.join("app.rc");
    std::fs::write(&rc_path, rc).unwrap();
    let obj = out.join("app_res.o");
    let candidates: Vec<String> = std::env::var("WINDRES")
        .ok()
        .into_iter()
        .chain(["x86_64-w64-mingw32-windres".to_string(), "windres".to_string()])
        .collect();
    for windres in candidates {
        let status = Command::new(&windres).arg("-O").arg("coff").arg("-i").arg(&rc_path).arg("-o").arg(&obj).status();
        if matches!(status, Ok(s) if s.success()) {
            println!("cargo:rustc-link-arg-bins={}", obj.display());
            return;
        }
    }
    println!("cargo:warning=windres not found: the .exe gets no icon (set WINDRES to MinGW's windres)");
}
