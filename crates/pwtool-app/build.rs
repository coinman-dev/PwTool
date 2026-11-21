// Embeds into the library the icons extracted from the original forms
// (assets/icons, `icons::icon_bytes`) and the program data
// (`resources::FILES`): translations, common passwords, script examples,
// license and, if they have been built, the PDF manuals (target/manuals or
// $PWTOOL_MANUALS_DIR; see packaging/manuals.sh).
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
    resources(Path::new(&manifest));
}

/// `resources.rs`: `&[(path relative to the data directory, contents)]`.
fn resources(manifest: &Path) {
    let assets = manifest.join("../../assets");
    let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
    for name in ["common_passwords.txt", "license.txt"] {
        files.push((name.into(), assets.join(name)));
    }
    let mut dir = |rel: &str, exts: &[&str]| {
        let d = assets.join(rel);
        println!("cargo:rerun-if-changed={}", d.display());
        let mut found: Vec<_> = std::fs::read_dir(&d)
            .unwrap_or_else(|e| panic!("{}: {e}", d.display()))
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| exts.iter().any(|e| x.eq_ignore_ascii_case(e))))
            .collect();
        found.sort();
        for p in found {
            files.push((format!("{rel}/{}", p.file_name().unwrap().to_string_lossy()), p));
        }
    };
    dir("languages", &["po", "lng", "pot"]);
    dir("languages/port", &["po"]);
    dir("script_examples", &["lua"]);
    // PDF manuals: built from doc/manual/*.odt with LibreOffice before the build
    println!("cargo:rerun-if-env-changed=PWTOOL_MANUALS_DIR");
    let manuals = std::env::var_os("PWTOOL_MANUALS_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| manifest.join("../../target/manuals"));
    for name in ["manual.pdf", "scripting.pdf"] {
        let p = manuals.join(name);
        println!("cargo:rerun-if-changed={}", p.display());
        if p.is_file() {
            files.push((name.into(), p));
        } else if std::env::var("PROFILE").as_deref() == Ok("release") {
            println!("cargo:warning=manual not embedded: {} missing (run packaging/manuals.sh)", p.display());
        }
    }
    let mut code = String::from("&[\n");
    for (rel, p) in &files {
        println!("cargo:rerun-if-changed={}", p.display());
        let abs = p.canonicalize().unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        writeln!(code, "    ({rel:?}, include_bytes!({:?})),", abs.display().to_string()).unwrap();
    }
    code.push_str("]\n");
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("resources.rs"), code).unwrap();
}
