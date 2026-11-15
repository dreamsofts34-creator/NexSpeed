fn main() {
    // The app uses Slint's software renderer, so images (the window icon) are decoded at build
    // time and embedded as raw pixels: no image-decoding code at runtime.
    let config = slint_build::CompilerConfiguration::new()
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config("ui/overlay.slint", config).expect("failed to compile Slint UI");
}
