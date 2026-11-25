fn main() {
    println!("cargo:rerun-if-changed=../../panel/frontend/dist");
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../installer/assets/app.ico");
        let _ = res.compile();
    }
}
