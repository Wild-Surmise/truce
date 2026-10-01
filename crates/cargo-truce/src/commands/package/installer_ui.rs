//! Native wizard copy, shared by per-product and suite Windows installers.

use crate::config::WindowsPackagingConfig;
use std::fmt::Write;

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
    fn product_copy_overrides_only_configured_fields() {
        let overrides = crate::config::InstallerConfig {
            welcome_html: Some("moon.html".into()),
            completion_message: Some("Echoes!".into()),
            ..Default::default()
        };
        let macos = crate::config::MacosPackagingConfig {
            welcome_html: Some("default.html".into()),
            conclusion_html: Some("conclusion.html".into()),
            license_html: Some("license.html".into()),
            ..Default::default()
        };
        let resolved = overrides.macos_packaging(&macos);
        assert_eq!(resolved.welcome_html.as_deref(), Some("moon.html"));
        assert_eq!(resolved.conclusion_html, macos.conclusion_html);
        assert_eq!(resolved.license_html, macos.license_html);
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
    fn macos_conclusion_is_opt_in_and_has_an_explicit_content_type() {
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
        config.conclusion_html = Some("assets/conclusion.html".into());
        assert!(
            render(&config)
                .contains("<conclusion file=\"conclusion.html\" mime-type=\"text/html\"/>")
        );
    }
}
