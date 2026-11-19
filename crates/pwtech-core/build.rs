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
}
