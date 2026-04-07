//! Test Username detection in GraphQL type mapper

use mockpit_graphql::TypeToFakeMapper;

fn main() {
  let mapper = TypeToFakeMapper::new();

  println!("Testing GraphQL scalar to fake mapping with Username detection:\n");

  // Test String scalar with login field name
  let result = mapper.scalar_to_fake_with_field("String", Some("login"));
  println!("Field: login, GraphQL Type: String");
  println!("Generated: {}\n", result);

  // Test String scalar with username field name
  let result = mapper.scalar_to_fake_with_field("String", Some("username"));
  println!("Field: username, GraphQL Type: String");
  println!("Generated: {}\n", result);

  // Test String scalar with name field name (should be different)
  let result = mapper.scalar_to_fake_with_field("String", Some("name"));
  println!("Field: name, GraphQL Type: String");
  println!("Generated: {}\n", result);

  // Test without field name (should fall back to String default)
  let result = mapper.scalar_to_fake("String");
  println!("No field name, GraphQL Type: String");
  println!("Generated: {}\n", result);
}
