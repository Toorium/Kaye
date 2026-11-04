//! Puts Kaye's icon inside the .exe, so Explorer and pinned taskbar icons show it.

fn main() {
    println!("cargo:rerun-if-changed=assets/kaye.ico");
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/kaye.ico");
        // A missing resource compiler shouldn't stop Kaye from building. The window
        // still gets its icon at runtime.
        if let Err(error) = resource.compile() {
            println!("cargo:warning=Couldn't embed the .exe icon: {error}");
        }
    }
}
