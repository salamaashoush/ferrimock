//! Core mock engine for HTTP request matching and response generation
//!
//! Core mocking engine providing:
//! - Pattern-based request matching
//! - Static response generation
//! - Priority-based mock selection
//! - YAML/JSON configuration support

pub mod har_export;
pub mod matcher;
pub mod patcher;
pub mod recorder_ext;
pub mod registry;
pub mod request_patcher;
pub mod scope;
pub mod types;
pub mod validation;

// Export only bdg-mock-engine specific types
pub use har_export::export_mocks_to_har;
pub use matcher::{MockAction, MockMatch, MockMatcher};
pub use patcher::ResponsePatcher;
pub use recorder_ext::MockRecorderConsolidationExt;
pub use registry::MockRegistry;
pub use request_patcher::RequestPatcher;
pub use scope::{ScopeInfo, ScopeManager};
pub use types::{
    BodyMatcher, BodySource, HeaderMatcher, MockDefinition, PatchOperation, QueryMatcher,
    RequestContext, RequestMatcher, ResponseGenerator, ResponseGeneratorExt, ResponseMode,
    UrlPattern,
};
pub use validation::{
    CodeSnippet, ErrorType, MockValidator, ValidationError, ValidationResult, ValidationWarning,
    WarningType,
};
