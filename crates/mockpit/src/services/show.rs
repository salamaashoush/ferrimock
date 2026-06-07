//! Show mock detail service.

use crate::engine::MockRegistry;
use crate::types::MockDefinition;
use std::sync::Arc;

/// Show a single mock definition by ID.
pub async fn show(
    mock_id: &str,
    mocks_dir: Option<&str>,
) -> Result<Option<Arc<MockDefinition>>, crate::MockpitError> {
    let default_dir = std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string());
    let dir = mocks_dir.unwrap_or(&default_dir);

    let registry = MockRegistry::new();
    registry
        .load_from_directory(dir)
        .await
        .map_err(|e| crate::mp_err!(e))?;

    Ok(registry.get_mock(mock_id))
}

/// Show a mock from an existing registry.
pub fn show_from_registry(
    registry: &MockRegistry,
    mock_id: &str,
) -> Option<Arc<MockDefinition>> {
    registry.get_mock(mock_id)
}
