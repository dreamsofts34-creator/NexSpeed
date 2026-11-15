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
        if let Err(e) = res.compile() {
            // Never block a build just because the icon could not be embedded.
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
