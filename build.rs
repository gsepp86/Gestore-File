// Incorpora icona e informazioni nel file .exe (solo su Windows).
// Se fallisce, la compilazione continua lo stesso.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Gestore File CP");
        res.set("FileDescription", "Gestore File");
        res.set("CompanyName", "Peppe CP");
        res.set("LegalCopyright", "Creato da Peppe CP con l'aiuto della IA");
        res.set("ProductVersion", "1.0");
        res.set("FileVersion", "1.0");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Icona non incorporata: {e}");
        }
    }
}
