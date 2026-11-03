//! Native installer copy and artwork shared by product and suite installers.

use crate::config::WindowsPackagingConfig;
use std::fmt::Write;

pub(super) fn macos_background_xml(
    config: Option<&crate::config::MacosPackagingConfig>,
) -> &'static str {
    if config.and_then(|c| c.background_png.as_deref()).is_none() {
        return "";
    }
    "    <background file=\"background.png\" mime-type=\"image/png\" alignment=\"bottomleft\" scaling=\"none\"/>\n\
     <background-darkAqua file=\"background.png\" mime-type=\"image/png\" alignment=\"bottomleft\" scaling=\"none\"/>\n"
}

#[cfg(target_os = "macos")]
pub(super) fn stage_macos_background(
    root: &std::path::Path,
    resources: &std::path::Path,
    config: &crate::config::MacosPackagingConfig,
) -> std::io::Result<()> {
    if let Some(path) = &config.background_png {
        std::fs::copy(root.join(path), resources.join("background.png"))?;
    }
    Ok(())
}

pub(super) fn write_windows_setup(out: &mut String, config: &WindowsPackagingConfig) {
    // Inno hides the welcome page by default, even with custom label text.
    if config.welcome_message.is_some() {
        out.push_str("DisableWelcomePage=no\r\n");
    }
}

pub(super) fn write_windows_messages(out: &mut String, config: &WindowsPackagingConfig) {
    if config.welcome_message.is_none() && config.completion_message.is_none() {
        return;
    }
    out.push_str("\r\n[Messages]\r\n");
    for (key, text) in [
        ("WelcomeLabel2", &config.welcome_message),
        ("FinishedLabel", &config.completion_message),
    ] {
        if let Some(text) = text {
            // Literal percent signs must not become Inno message arguments.
            let text = text
                .replace('%', "%%")
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .replace('\n', "%n");
            let _ = write!(out, "{key}={text}\r\n");
        }
    }
    out.push_str("\r\n[Code]\r\nprocedure InitializeWizard;\r\nbegin\r\n");
    for (label, text) in [
        ("WelcomeLabel2", &config.welcome_message),
        ("FinishedLabel", &config.completion_message),
    ] {
        if text.is_some() {
            let _ = write!(out, "  WizardForm.{label}.Font.Name := 'Courier New';\r\n");
        }
    }
    out.push_str("end;\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_welcome_enables_page_for_product_and_suite() {
        let vendor = WindowsPackagingConfig {
            welcome_message: Some("Welcome to the suite".into()),
            ..Default::default()
        };
        let product = crate::config::InstallerConfig {
            welcome_message: Some("Welcome to this product".into()),
            ..Default::default()
        };
        for config in [
            vendor.clone(),
            product.windows_packaging(&vendor),
            product.windows_packaging(&WindowsPackagingConfig::default()),
            crate::config::InstallerConfig::default().windows_packaging(&vendor),
        ] {
            let mut out = String::new();
            write_windows_setup(&mut out, &config);
            assert_eq!(out, "DisableWelcomePage=no\r\n");
        }
    }

    #[test]
    fn absent_welcome_preserves_default_even_with_completion_copy() {
        for completion_message in [None, Some("Finished".into())] {
            let config = WindowsPackagingConfig {
                completion_message,
                ..Default::default()
            };
            let mut out = String::new();
            write_windows_setup(&mut out, &config);
            assert!(out.is_empty());
        }
    }

    #[test]
    fn product_copy_overrides_only_configured_fields() {
        let overrides = crate::config::InstallerConfig {
            welcome_html: Some("moon.html".into()),
            completion_message: Some("Echoes!".into()),
            ..Default::default()
        };
        let macos = crate::config::MacosPackagingConfig {
            background_png: Some("brand.png".into()),
            welcome_html: Some("default.html".into()),
            conclusion_html: Some("conclusion.html".into()),
            license_html: Some("license.html".into()),
            ..Default::default()
        };
        let resolved = overrides.macos_packaging(&macos);
        assert_eq!(resolved.welcome_html.as_deref(), Some("moon.html"));
        assert_eq!(resolved.conclusion_html, macos.conclusion_html);
        assert_eq!(resolved.license_html, macos.license_html);
        assert_eq!(resolved.background_png, macos.background_png);
        let windows = WindowsPackagingConfig {
            welcome_message: Some("Hello!".into()),
            completion_message: Some("Goodbye!".into()),
            license_rtf: Some("license.txt".into()),
            ..Default::default()
        };
        let resolved = overrides.windows_packaging(&windows);
        assert_eq!(resolved.welcome_message, windows.welcome_message);
        assert_eq!(resolved.completion_message.as_deref(), Some("Echoes!"));
        assert_eq!(resolved.license_rtf, windows.license_rtf);
    }

    #[test]
    fn messages_preserve_literal_percent_signs_and_line_breaks() {
        let config = WindowsPackagingConfig {
            welcome_message: Some("100% music\r\n\r\nTom.".into()),
            completion_message: Some("Have fun!\nTom.".into()),
            ..Default::default()
        };
        let mut out = String::new();
        write_windows_messages(&mut out, &config);
        assert!(out.contains("WelcomeLabel2=100%% music%n%nTom.\r\n"));
        assert!(out.contains("FinishedLabel=Have fun!%nTom.\r\n"));
        assert!(out.contains("WizardForm.WelcomeLabel2.Font.Name := 'Courier New';"));
        assert!(out.contains("WizardForm.FinishedLabel.Font.Name := 'Courier New';"));
    }

    #[test]
    fn unconfigured_installers_keep_native_messages() {
        let mut out = String::new();
        write_windows_messages(&mut out, &WindowsPackagingConfig::default());
        assert!(out.is_empty());
    }

    #[test]
    fn macos_artwork_and_conclusion_are_opt_in_with_explicit_content_types() {
        let mut config = crate::config::MacosPackagingConfig::default();
        let render = |config: &crate::config::MacosPackagingConfig| {
            super::super::stage::generate_distribution_xml(
                "Example",
                "com.example",
                "example",
                &[super::super::PkgFormat::Clap],
                &[],
                "1.0.0",
                Some(config),
                crate::install_scope::PkgScope::User,
                false,
            )
        };
        assert!(!render(&config).contains("<conclusion"));
        assert!(!render(&config).contains("<background"));
        config.conclusion_html = Some("assets/conclusion.html".into());
        config.background_png = Some("assets/brand.png".into());
        assert!(render(&config).contains(macos_background_xml(Some(&config))));
        assert!(render(&config).contains("<background-darkAqua"));
        assert!(
            render(&config)
                .contains("<conclusion file=\"conclusion.html\" mime-type=\"text/html\"/>")
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_artwork_is_staged_and_missing_artwork_fails() {
        let root = std::env::temp_dir().join(format!(
            "truce-installer-artwork-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let resources = root.join("resources");
        std::fs::create_dir_all(&resources).unwrap();
        let mut config = crate::config::MacosPackagingConfig::default();
        stage_macos_background(&root, &resources, &config).unwrap();
        assert!(!resources.join("background.png").exists());
        config.background_png = Some("logo.png".into());
        assert!(stage_macos_background(&root, &resources, &config).is_err());
        std::fs::write(root.join("logo.png"), b"logo bytes").unwrap();
        stage_macos_background(&root, &resources, &config).unwrap();
        assert_eq!(
            std::fs::read(resources.join("background.png")).unwrap(),
            b"logo bytes"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
