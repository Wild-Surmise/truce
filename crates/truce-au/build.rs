fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "macos" {
        return;
    }

    let out_dir = std::env::var("OUT_DIR").unwrap();

    println!("cargo:rerun-if-env-changed=TRUCE_AU_VERSION");
    println!("cargo:rerun-if-changed=shim/au_shim.m");
    println!("cargo:rerun-if-changed=shim/au_v2_shim.c");
    println!("cargo:rerun-if-changed=shim/au_shim_common.c");
    let shim_include = truce_shim_types::include_dir();
    println!(
        "cargo:rerun-if-changed={}",
        shim_include.join("au_shim_types.h").display()
    );

    // TRUCE_AU_VERSION=2 builds only the v2 C shim (for .component)
    // TRUCE_AU_VERSION=3 builds both v3 ObjC shim AND v2 C shim (for .appex).
    //   The v2 shim is needed so Apple's v3→v2 bridge works — hosts call
    //   AudioComponentCopyConfigurationInfo (sync) during scanning, which uses
    //   the v2 bridge internally to query channel configs, formats, etc.
    // Default (unset): builds both.
    let au_version = std::env::var("TRUCE_AU_VERSION").unwrap_or_default();

    // The v3 ObjC subclass + factory class still live in au_shim.m.
    // They use C `#define`d names so they're unique per plugin (ObjC
    // class names are process-global and would collide between
    // plugins loaded into the same host). The v2 cocoa view factory
    // moved to runtime ObjC class registration in `cocoa_view.rs`,
    // so we no longer set its `-DTRUCE_AU_VIEW_FACTORY_NAME` here and
    // no longer compile `au_v2_view.m`.
    let compiles_v3_shim = au_version != "2" && au_version != "3";

    let mut build = cc::Build::new();

    // Common globals + registration — always compiled.
    build.file("shim/au_shim_common.c");

    // V2 shim — always compiled. Pure C now (no per-plugin defines
    // since the cocoa view factory class name comes from the Rust
    // function `truce_au_view_factory_class_name`).
    build.file("shim/au_v2_shim.c");

    if compiles_v3_shim {
        // Only the v3 ObjC subclass file consumes the per-plugin
        // class-name defines, so the env-var read and the defines
        // are scoped to the v3 build path. v2-only builds (the
        // common case for `.component` plugins) are now plugin-id
        // agnostic and `truce-au` compiles once per arch across all
        // plugins.
        println!("cargo:rerun-if-env-changed=TRUCE_AU_PLUGIN_ID");

        let plugin_id = std::env::var("TRUCE_AU_PLUGIN_ID").unwrap_or_default();
        let sanitized: String = if plugin_id.is_empty() {
            "default".to_string()
        } else {
            plugin_id
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect()
        };
        let class_name = format!("TruceAU_{sanitized}");
        let factory_class_name = format!("TruceAUFactory_{sanitized}");

        build.file("shim/au_shim.m");
        build
            .define("TRUCE_AU_CLASS_NAME", class_name.as_str())
            .define("TRUCE_AU_FACTORY_CLASS_NAME", factory_class_name.as_str());

        // Reference the ObjC class symbols directly so the consumer's
        // dead-stripping pass keeps them. v3 appex builds skip this
        // path because the ObjC classes live in the appex binary, not
        // the framework dylib.
        println!("cargo:rustc-link-arg-cdylib=-Wl,-u,_OBJC_CLASS_$_{class_name}");
        println!("cargo:rustc-link-arg-cdylib=-Wl,-u,_OBJC_CLASS_$_{factory_class_name}");
    }

    build
        .include(&shim_include)
        .flag("-fobjc-arc")
        .flag("-fmodules")
        .flag("-fvisibility=default")
        .flag("-mmacosx-version-min=11.0");

    build.compile("au_shim");

    // `rustc-link-arg-cdylib` propagates to the downstream cdylib that
    // consumes us (per cargo issue 9562) so the C shim gets force-loaded
    // and AU entry symbols (g_descriptor / TruceAUFactory / etc.)
    // survive dead-stripping in the consumer's plugin dylib. We can't
    // host our own cdylib target here because the exported symbols are
    // defined by the `export_au!` macro in the consuming crate.
    println!("cargo:rustc-link-arg-cdylib=-Wl,-force_load,{out_dir}/libau_shim.a");

    // Export shim globals so the appex binary can access them from the framework.
    // The appex compiles the ObjC classes separately and needs these at runtime.
    println!("cargo:rustc-link-arg-cdylib=-Wl,-exported_symbol,_g_descriptor");
    println!("cargo:rustc-link-arg-cdylib=-Wl,-exported_symbol,_g_callbacks");
    println!("cargo:rustc-link-arg-cdylib=-Wl,-exported_symbol,_g_param_descriptors");
    println!("cargo:rustc-link-arg-cdylib=-Wl,-exported_symbol,_g_num_params");

    // Tell Rust code which AU version we're building
    if au_version == "3" {
        println!("cargo:rustc-cfg=truce_au_v3_only");
    }

    // Always export the v2 factory symbol — hosts use v2 API to instantiate.
    println!("cargo:rustc-link-arg-cdylib=-Wl,-exported_symbol,_TruceAUFactory");

    // Cocoa view factory class name lookup is provided by the Rust
    // side (`cocoa_view::truce_au_view_factory_class_name`). Force its
    // symbol to be exported so `au_v2_shim.c`'s call to it links.
    println!("cargo:rustc-link-arg-cdylib=-Wl,-exported_symbol,_truce_au_view_factory_class_name");

    println!("cargo:rustc-link-lib=framework=AudioToolbox");
    println!("cargo:rustc-link-lib=framework=AVFAudio");
    println!("cargo:rustc-link-lib=framework=CoreAudio");
    println!("cargo:rustc-link-lib=framework=CoreMIDI");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=AppKit");
}
