//! OpenAPI front end: a document in, an entity graph out.

pub mod document;
pub mod entities;
pub mod schema;

pub use document::{
    DefectKind, OpenApiVersion, Operation, OperationTable, ParamIn, Parameter, ResponseSpec,
    SchemaBook, SchemaKind, SchemaNode, SchemaRef, Segment, SpecDefect, parse_openapi,
};
pub use entities::{Inference, to_entity_graph, to_entity_graph_with};
