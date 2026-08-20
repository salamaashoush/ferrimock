//! Deciding what an OpenAPI operation *does*, at build time.
//!
//! REST says most of it out loud: the method is the verb and the path shape
//! says whether one thing or many are addressed. What it does not say is which
//! entity, which is what [`operation_target`] works out — so this is short
//! where the GraphQL classifier is long, and lands in the same [`RootPlan`].

use lean_string::LeanString;

use crate::core::world::model::EntityGraph;
use crate::spec::bind::plan::RootPlan;
use crate::spec::infer::openapi::document::{Operation, OperationTable, Segment};
use crate::spec::infer::openapi::entities::{Target, operation_target, subresource_parent};

/// Classify one operation.
#[must_use]
pub fn classify(table: &OperationTable, operation: &Operation, graph: &EntityGraph) -> RootPlan {
    let Some(target) = operation_target(table, operation, graph) else {
        return RootPlan::Unclassified;
    };

    let key_arg = item_key(operation);
    let input_arg = body_envelope(table, operation, graph);

    let method = &operation.method;
    // `GET /folders` and `GET /folders/{id}` differ only in the path, and a
    // response that is a list settles it either way.
    if *method == http::Method::GET {
        classify_read(&target, key_arg)
    } else if *method == http::Method::POST {
        if !creates(operation, graph, &target) {
            return RootPlan::Unclassified;
        }
        RootPlan::Create {
            entity: target.entity,
            input_arg,
            payload_field: target.payload_field,
        }
    } else if *method == http::Method::PUT || *method == http::Method::PATCH {
        key_arg.map_or(RootPlan::Unclassified, |key_arg| RootPlan::Update {
            entity: target.entity,
            key_arg,
            input_arg,
            payload_field: target.payload_field,
        })
    } else if *method == http::Method::DELETE {
        key_arg.map_or(RootPlan::Unclassified, |key_arg| RootPlan::Delete {
            entity: target.entity,
            key_arg,
            payload_field: target.payload_field,
        })
    } else {
        RootPlan::Unclassified
    }
}

fn classify_read(target: &Target, key_arg: Option<LeanString>) -> RootPlan {
    if target.is_list {
        return RootPlan::List {
            entity: target.entity.clone(),
            members: target.members.clone(),
            connection: None,
            payload_field: target.payload_field.clone(),
        };
    }

    // A path that addresses nothing — `/me`, `/users/current` — is a read of
    // one instance with no key to read it by, which is the one endpoint whose
    // whole purpose is to answer differently per caller.
    let Some(key_arg) = key_arg else {
        return RootPlan::Viewer {
            entity: target.entity.clone(),
            members: target.members.clone(),
        };
    };

    RootPlan::Get {
        entity: target.entity.clone(),
        members: target.members.clone(),
        key_arg,
    }
}

/// Whether a POST creates the thing its response describes.
///
/// A collection path does: `POST /folders` makes a folder, and so does
/// `POST /folders/{id}/items` because `items` is a sub-collection the graph
/// knows. `POST /folders/{id}/copy` does not — `copy` is an action on one
/// folder, and answering it by inserting a record would be a fabrication that
/// also leaves the world wrong.
fn creates(operation: &Operation, graph: &EntityGraph, target: &Target) -> bool {
    if target.is_list || matches!(operation.segments.last(), Some(Segment::Param(_))) {
        return false;
    }
    if operation.path_params().next().is_none() {
        return true;
    }
    subresource_parent(graph, operation, Some(&target.entity)).is_some()
}

/// The path parameter addressing one instance.
fn item_key(operation: &Operation) -> Option<LeanString> {
    match operation.segments.last() {
        Some(Segment::Param(name)) => Some(name.clone()),
        _ => None,
    }
}

/// The field a request body wraps the entity in, when it wraps it.
fn body_envelope(
    table: &OperationTable,
    operation: &Operation,
    graph: &EntityGraph,
) -> Option<LeanString> {
    let is_entity = |name: &str| graph.contains(name);
    crate::spec::infer::openapi::entities::target_of(
        &table.schemas,
        operation.request_body.as_ref()?,
        &is_entity,
    )?
    .payload_field
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::spec::infer::openapi::document::parse_openapi;
    use crate::spec::infer::openapi::entities::to_entity_graph;

    const DOC: &str = r#"
openapi: 3.0.3
info: { title: t }
paths:
  /folders:
    get:
      operationId: listFolders
      responses:
        "200":
          content:
            application/json:
              schema:
                type: object
                properties:
                  entries:
                    type: array
                    items: { $ref: '#/components/schemas/Folder' }
                  total_count: { type: integer }
    post:
      operationId: createFolder
      requestBody:
        content:
          application/json:
            schema: { $ref: '#/components/schemas/Folder' }
      responses:
        "201":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
  /folders/{folder_id}:
    parameters:
      - { name: folder_id, in: path, required: true, schema: { type: string } }
    get:
      operationId: getFolder
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
    put:
      operationId: replaceFolder
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
    patch:
      operationId: updateFolder
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
    delete:
      operationId: deleteFolder
      responses:
        "204": { description: gone }
  /folders/{folder_id}/copy:
    post:
      operationId: copyFolder
      parameters:
        - { name: folder_id, in: path, required: true, schema: { type: string } }
      responses:
        "201":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
  /folders/{folder_id}/items:
    get:
      operationId: listFolderItems
      parameters:
        - { name: folder_id, in: path, required: true, schema: { type: string } }
      responses:
        "200":
          content:
            application/json:
              schema:
                type: array
                items: { $ref: '#/components/schemas/Folder' }
  /me:
    get:
      operationId: getCurrentUser
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/User' }
  /health:
    get:
      operationId: health
      responses:
        "200":
          content:
            application/json:
              schema: { type: object, properties: { ok: { type: boolean } } }
components:
  schemas:
    Folder:
      type: object
      required: [id]
      properties:
        id: { type: string }
        name: { type: string }
    User:
      type: object
      properties:
        id: { type: string }
"#;

    fn plan(id: &str) -> RootPlan {
        let table = parse_openapi(DOC).unwrap().0;
        let graph = to_entity_graph(&table);
        let operation = table.operation(id).expect("the operation exists");
        classify(&table, operation, &graph)
    }

    #[test]
    fn a_collection_path_is_a_list() {
        let RootPlan::List {
            entity,
            payload_field,
            ..
        } = plan("listFolders")
        else {
            panic!("listFolders should be a list")
        };
        assert_eq!(entity.as_str(), "Folder");
        assert_eq!(payload_field.as_deref(), Some("entries"));
    }

    #[test]
    fn an_item_path_is_a_lookup_keyed_by_its_parameter() {
        let RootPlan::Get {
            entity, key_arg, ..
        } = plan("getFolder")
        else {
            panic!("getFolder should be a lookup")
        };
        assert_eq!(entity.as_str(), "Folder");
        assert_eq!(key_arg.as_str(), "folder_id");
    }

    /// `/me` is a read of one instance with no key to read it by, which makes
    /// it the caller rather than a lookup that lost its argument.
    #[test]
    fn a_path_addressing_nothing_reads_the_caller() {
        let RootPlan::Viewer { entity, .. } = plan("getCurrentUser") else {
            panic!("/me should be the caller")
        };
        assert_eq!(entity.as_str(), "User");
    }

    #[test]
    fn the_methods_land_on_their_rungs() {
        assert_eq!(plan("createFolder").rung(), "create");
        assert_eq!(plan("replaceFolder").rung(), "update");
        assert_eq!(plan("updateFolder").rung(), "update");
        assert_eq!(plan("deleteFolder").rung(), "delete");
        assert_eq!(plan("listFolderItems").rung(), "list");
    }

    #[test]
    fn a_post_to_an_item_path_is_an_action_not_a_creation() {
        assert_eq!(
            plan("copyFolder"),
            RootPlan::Unclassified,
            "`/folders/{{id}}/copy` acts on a folder; it does not create one"
        );
    }

    #[test]
    fn a_response_that_is_not_an_entity_drops_to_the_bottom_rung() {
        assert_eq!(plan("health"), RootPlan::Unclassified);
        assert!(!RootPlan::Unclassified.is_classified());
    }
}
