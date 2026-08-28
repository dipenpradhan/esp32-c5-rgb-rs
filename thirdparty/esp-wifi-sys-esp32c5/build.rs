use std::{env, path::PathBuf};

fn main() {
    // Put the linker script somewhere the linker can find it
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    let libs = [
        "ble_app",
        "btbb",
        "coexist",
        "core",
        "espnow",
        "mesh",
        "net80211",
        "phy",
        "pp",
        "smartconfig",
        "wapi",
        "wpa_supplicant",
        "printf",
        "regulatory",
    ];

    for lib in libs {
        std::fs::copy(
            format!("libs/lib{}.a", lib),
            out.join(format!("lib{}.a", lib)),
        )
        .unwrap_or_else(|e| panic!("Failed to copy the {lib} library: {e}"));
        println!("cargo:rustc-link-lib={lib}");
    }

    // Copy FTM stubs (satisfies linker references from newer libnet80211.a)
    // FTM (Fine Timing Measurement) is a WiFi 6 feature not needed for basic WiFi STA
    std::fs::copy("libs/libftm_stubs.a", out.join("libftm_stubs.a"))
        .unwrap_or_else(|e| panic!("Failed to copy libftm_stubs.a: {e}"));
    println!("cargo:rustc-link-lib=static=ftm_stubs");

    println!("cargo:rustc-link-search={}", out.display());
}
