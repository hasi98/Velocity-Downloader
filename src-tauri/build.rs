fn main() {
    slint_build::compile("ui/native_main.slint").expect("Failed to compile Slint UI");
    tauri_build::build()
}
