fn main() {
    #[cfg(windows)]
    winresource::WindowsResource::new().set_icon("packaging/serafin.ico").set("ProductName", "Serafin").compile().expect("embed exe icon");
}
