// Embeds the app icon (and version info from Cargo.toml) into nexspeed.exe on Windows,
// so Explorer, the taskbar and Start show it. No effect on other platforms.
fn main() {
    println!("cargo:rerun-if-changed=assets/nexspeed.ico");
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/nexspeed.ico");
        res.set("ProductName", "Nex Speed");
        res.set("FileDescription", "Nex Speed - internet speed meter");
        res.set("InternalName", "nexspeed");
        res.set("OriginalFilename", "nexspeed.exe");
        res.set("LegalCopyright", "Copyright (c) Nex Speed contributors. GPL-3.0-or-later.");
        if let Err(e) = res.compile() {
            // Never block a build just because the icon could not be embedded.
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
