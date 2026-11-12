//! Windows'ta uygulama simgesini (assets/logo/openify.ico) exe'ye kaynak olarak
//! gömer. embed-resource gpui'nin de kullandığı sürüm (yeni indirme yok);
//! Windows SDK'nın rc.exe'sini kendisi bulur.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/logo/openify.rc");
    println!("cargo:rerun-if-changed=assets/logo/openify.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/logo/openify.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("uygulama simgesi kaynağı derlenemedi");
    }
}
