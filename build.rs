fn main() {
    #[cfg(feature = "spine")]
    {
        let mut build = cc::Build::new();
        let directory = "vendor/spine-c/src/spine";
        let mut files: Vec<_> = std::fs::read_dir(directory)
            .expect("vendored Spine source directory is missing")
            .map(|entry| entry.expect("read Spine source entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
            .collect();
        files.sort();
        build
            .files(files)
            .file("src/spine_bridge.c")
            .include("vendor/spine-c/include")
            .warnings(false)
            .compile("arkpets_spine38");
        println!("cargo:rerun-if-changed=src/spine_bridge.c");
        println!("cargo:rerun-if-changed=vendor/spine-c");
    }
}
