use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object, namespace = "services")]
pub struct JsFakeDataInput {
    pub generator: String,
    pub count: Option<u32>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub words: Option<u32>,
    pub length: Option<u32>,
}

#[napi(object, namespace = "services")]
pub struct JsGeneratorInfo {
    pub name: String,
    pub category: String,
    pub description: String,
    pub example: String,
}

#[napi(namespace = "services")]
pub fn fake_data(input: JsFakeDataInput) -> Result<Vec<String>> {
    mockpit::services::fake_data::generate(mockpit::services::fake_data::FakeDataInput {
        generator: input.generator,
        count: input.count.unwrap_or(1) as usize,
        min: input.min,
        max: input.max,
        words: input.words.map(|v| v as usize),
        length: input.length.map(|v| v as usize),
    })
    .map_err(|e| Error::from_reason(e.to_string()))
}

#[napi(namespace = "services")]
pub fn list_generators(category: Option<String>, search: Option<String>) -> Vec<JsGeneratorInfo> {
    mockpit::services::fake_data::list_generators(category.as_deref(), search.as_deref())
        .into_iter()
        .map(|g| JsGeneratorInfo {
            name: g.name,
            category: g.category,
            description: g.description,
            example: g.example,
        })
        .collect()
}
