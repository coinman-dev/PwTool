// Embeds the icons extracted from the original forms (assets/icons) into
// the executable. For Windows also the program icon and version
// information as a resource (compiled with windres).
use std::fmt::Write;
use std::path::Path;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");
    println!("cargo:rerun-if-changed={}", root.display());
    for d in ["Main", "PasswManager"] {
        for c in ["ImageCollection16", "ImageCollection32"] {
            println!("cargo:rerun-if-changed={}", root.join(d).join(c).display());
        }
    }
    let mut entries = Vec::new();
    if let Ok(forms) = std::fs::read_dir(&root) {
        for f in forms.flatten() {
            for coll in std::fs::read_dir(f.path()).into_iter().flatten().flatten() {
                for img in std::fs::read_dir(coll.path()).into_iter().flatten().flatten() {
                    let p = img.path();
                    if p.extension().is_some_and(|e| e == "png") {
                        let key = format!(
                            "{}/{}/{}",
                            f.file_name().to_string_lossy(),
                            coll.file_name().to_string_lossy(),
                            p.file_stem().unwrap().to_string_lossy()
                        );
                        println!("cargo:rerun-if-changed={}", p.display());
                        entries.push((key, p.canonicalize().unwrap()));
                    }
                }
            }
        }
    }
    entries.sort();
    let mut code = String::from("pub fn icon_bytes(key: &str) -> Option<&'static [u8]> {\n    match key {\n");
    for (k, p) in &entries {
        writeln!(code, "        {k:?} => Some(include_bytes!({:?})),", p.display().to_string()).unwrap();
    }
    code.push_str("        _ => None,\n    }\n}\n");
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("icons.rs"), code).unwrap();
}

/// Program icon (`pwtool.ico` with PNG images from assets/appicons) and
/// version information, compiled with windres (llvm-mingw / MinGW) and
/// linked into the executable. The first icon of the executable is also used
/// by GTK for its windows and the taskbar.
fn windows_resources() {
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).to_path_buf();
    let icons = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/appicons");
    let mut images = Vec::new();
    for s in [16, 32, 48, 64, 128, 256] {
        let p = icons.join(format!("pwtool_default_{s}.png"));
        println!("cargo:rerun-if-changed={}", p.display());
        let data = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        // width and height from the IHDR chunk
        let w = u32::from_be_bytes(data[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(data[20..24].try_into().unwrap());
        images.push((w, h, data));
    }
    // ICO file with PNG-compressed images (Windows Vista and later)
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (w, h, data) in &images {
        ico.push(if *w >= 256 { 0 } else { *w as u8 });
        ico.push(if *h >= 256 { 0 } else { *h as u8 });
        ico.extend_from_slice(&[0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, _, data) in &images {
        ico.extend_from_slice(data);
    }
    let ico_path = out.join("pwtool.ico");
    std::fs::write(&ico_path, ico).unwrap();

    let version = env!("CARGO_PKG_VERSION");
    let nums: Vec<u16> = version
        .split(['.', '-'])
        .take(3)
        .map(|x| x.parse().unwrap_or(0))
        .chain(std::iter::repeat(0))
        .take(4)
        .collect();
    let numeric = format!("{},{},{},{}", nums[0], nums[1], nums[2], nums[3]);
    let rc = format!(
        r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Password Tool"
      VALUE "FileDescription", "Password Tool"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "pwtool"
      VALUE "LegalCopyright", "GPL-2.0-or-later"
      VALUE "OriginalFilename", "pwtool.exe"
      VALUE "ProductName", "Password Tool"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        ico = ico_path.display().to_string().replace('\\', "/"),
    );
    let rc_path = out.join("pwtool.rc");
    std::fs::write(&rc_path, rc).unwrap();
    let obj = out.join("pwtool-res.o");
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".into());
    println!("cargo:rerun-if-env-changed=WINDRES");
    let ok = std::process::Command::new(&windres)
        .arg("-i")
        .arg(&rc_path)
        .arg("-O")
        .arg("coff")
        .arg("-o")
        .arg(&obj)
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        println!("cargo:rustc-link-arg-bins={}", obj.display());
    } else {
        println!("cargo:warning={windres} failed: the executable has no icon and version resource");
    }
}
