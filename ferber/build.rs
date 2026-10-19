use std::{env, path::Path};

fn main() {
    println!("cargo:rerun-if-env-changed=FERBER_USER_ELF");
    if env::var_os("CARGO_FEATURE_USERSPACE_ELF_PROBE").is_some() {
        let image = env::var("FERBER_USER_ELF")
            .expect("userspace-elf-probe requires FERBER_USER_ELF; run scripts/build-user-elf.sh");
        let path = Path::new(&image);
        assert!(
            path.is_absolute() && path.is_file(),
            "FERBER_USER_ELF must name an absolute ELF file"
        );
        println!("cargo:rerun-if-changed={image}");
    }
}
