fn main() {
    println!("cargo:rustc-link-lib=xenctrl");
    println!("cargo:rustc-link-lib=xenevtchn");
    println!("cargo:rustc-link-lib=xenforeignmemory");
    println!("cargo:rustc-link-lib=xendevicemodel");
    println!("cargo:rustc-link-lib=xencall");
    println!("cargo:rustc-link-lib=xengnttab");
    println!("cargo:rustc-link-lib=xentoolcore");
    println!("cargo:rustc-link-lib=xentoollog");
}