//! Embeds the exe's icon (`swisswheel.rc` → `assets/swisswheel.ico`).

fn main() {
    println!("cargo:rerun-if-changed=swisswheel.rc");
    println!("cargo:rerun-if-changed=../../assets/swisswheel.ico");
    embed_resource::compile("swisswheel.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("the icon resource compiles (needs rc.exe from the Windows SDK)");
}
