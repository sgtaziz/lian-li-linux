fn main() {
    // libevdi ships with a DKMS kernel module and a userspace .so.
    // No .pc file is installed by the Arch package as of libevdi 1.14, so we
    // link directly and let the dynamic loader find /usr/lib/libevdi.so.
    println!("cargo:rustc-link-lib=dylib=evdi");
}
