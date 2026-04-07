//! Mock management command
//!
//! Create, list, test, serve, and manage HTTP mock definitions.

use super::{MockAction, MockCommand};
use super::{
    consolidate, convert, create, export, format, list, recordings, reload, serve, show, test,
    validate, wizard,
};

/// Execute mock command
#[allow(clippy::large_futures)]
pub async fn execute(cmd: MockCommand) -> anyhow::Result<()> {
    match cmd.action {
        MockAction::Create {
            url,
            output,
            method,
            status,
            body,
            template,
            id,
            priority,
            collection,
            interactive,
        } => {
            match (interactive, url) {
                // Launch interactive wizard if --interactive flag is set or no URL provided
                (true, url) | (false, url @ None) => wizard::run_wizard(
                    url, output, &method, status, body, template, id, priority, collection,
                ),
                // Quick mode with flags when URL is provided
                (false, Some(url)) => create::create_mock(
                    output,
                    &method,
                    &url,
                    status,
                    body,
                    template,
                    id,
                    priority,
                    collection.as_deref(),
                    false,
                ),
            }
        }
        MockAction::List {
            collection,
            verbose,
        } => list::list_mocks(collection, verbose).await,
        MockAction::Show { mock_id } => show::show_mock(&mock_id).await,
        MockAction::Test {
            method,
            path,
            query,
            headers,
            body,
            render,
            debug,
            mock_file,
            json,
        } => {
            test::test_mock_match(test::TestMockParams {
                method_str: method,
                path,
                query,
                headers,
                body,
                render,
                debug,
                mock_file,
                json,
            })
            .await
        }
        MockAction::Reload { dir } => reload::reload_mocks(dir).await,
        MockAction::Recordings { dir } => recordings::list_recordings(dir),
        MockAction::Validate {
            path,
            format,
            stdin,
            file_format,
        } => validate::validate_mocks(path, &format, stdin, file_format).await,
        MockAction::Format {
            path,
            check,
            stdin,
            file_format,
        } => format::format_mocks(path, check, stdin, file_format.as_deref()),
        MockAction::Convert {
            input,
            output,
            format,
            matching: _,
            interactive,
            preflight,
            redirects,
            browser_headers,
            absolute_urls,
            domains,
            static_assets,
            keep_sensitive_headers,
            keep_infra_headers,
            extract_bodies,
            body_threshold_kb,
        } => {
            convert::convert_har(convert::ConvertHarOptions {
                input,
                output,
                format,
                interactive,
                exclude_preflight: !preflight,
                exclude_redirects: !redirects,
                strip_browser_headers: !browser_headers,
                normalize_urls: !absolute_urls,
                allowed_domains: domains,
                exclude_static_assets: !static_assets,
                strip_sensitive_headers: !keep_sensitive_headers,
                strip_infrastructure_headers: !keep_infra_headers,
                extract_bodies,
                body_threshold_kb,
            })
            .await
        }
        MockAction::Export {
            dir,
            output,
            collection,
        } => export::export_to_har(dir, output, collection).await,
        MockAction::Consolidate {
            input,
            output,
            format,
            min_pattern,
            no_templates,
            verbose,
        } => {
            consolidate::consolidate_mocks(
                input,
                output,
                format,
                min_pattern,
                !no_templates,
                verbose,
            )
            .await
        }
        MockAction::Serve {
            port,
            host,
            mocks,
            mock_file,
            watch,
            cors,
            enable_render_endpoint,
            log_matches,
            verbose,
            open,
        } => {
            serve::serve_mock_server(serve::MockServerConfig {
                port,
                host,
                mocks_dir: mocks,
                mock_file,
                watch,
                cors,
                enable_render_endpoint,
                log_matches,
                verbose,
                open_browser: open,
            })
            .await
        }
    }
}
