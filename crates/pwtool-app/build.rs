// Embeds the icons extracted from the original forms (assets/icons) into
// the library (`icons::icon_bytes`).
use std::fmt::Write;
use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let root = Path::new(&manifest).join("../../assets/icons");
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
