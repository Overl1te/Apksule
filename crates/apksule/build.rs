fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../assets/apksule.ico");
        res.set("ProductName", "Apksule");
        res.set("FileDescription", "Apksule — lightweight APK compatibility runtime");
        res.set("LegalCopyright", "Copyright (c) 2026 Apksule contributors");
        if let Err(error) = res.compile() {
            // Fail the build if the icon cannot be embedded: the installer and
            // explorer association rely on a branded executable.
            panic!("failed to embed Windows resources: {error}");
        }
    }
}
