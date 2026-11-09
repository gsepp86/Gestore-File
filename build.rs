// Incorpora l'icona nel file .exe (solo su Windows). Se fallisce, la compilazione continua lo stesso.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Icona non incorporata: {e}");
        }
    }
}
