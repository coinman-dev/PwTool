// Builds the C components that are taken unchanged from the original
// Password Tech sources (see csrc/*/ for their licences).

fn main() {
    println!("cargo:rerun-if-changed=csrc");

    // miniLZO 2.10 by Markus F.X.J. Oberhumer (GPL v2+), identical to
    // upstream/src/minilzo. Used for clipboard text encryption and for
    // estimating the entropy of user-provided data.
    cc::Build::new()
        .file("csrc/minilzo/minilzo.c")
        .include("csrc/minilzo")
        .warnings(false)
        .compile("minilzo");

    // miniz 3.0.0 by Rich Geldreich (MIT licence), identical to
    // upstream/src/miniz. Used for the compression of password databases,
    // so that compressed files are byte-identical to the original's.
    cc::Build::new()
        .file("csrc/miniz/miniz.c")
        .include("csrc/miniz")
        .define("MINIZ_NO_STDIO", None)
        .define("MINIZ_NO_TIME", None)
        .define("MINIZ_NO_ARCHIVE_APIS", None)
        .define("MINIZ_NO_ZLIB_COMPATIBLE_NAMES", None)
        .warnings(false)
        .compile("miniz");

    // zxcvbn-c by Tony Evans (MIT licence), identical to upstream/src/zxcvbn,
    // with the dictionary compiled in. Used for "advanced password strength
    // estimation".
    cc::Build::new()
        .cpp(true)
        .file("csrc/zxcvbn/zxcvbn.cpp")
        .include("csrc/zxcvbn")
        .warnings(false)
        .compile("zxcvbn");
}
