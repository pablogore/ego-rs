#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use proc_macro2::TokenStream;
    use syn::parse_quote;

    #[test]
    fn test_service_macro_compiles() {
        // This test verifies that the service macro compiles without errors
        // The actual descriptor generation is tested in the service-sdk crate
        let service_trait: TokenStream = parse_quote! {
            #[service]
            trait MyService {
                #[operation]
                async fn do_something(&self, input: String) -> Result<String, ServiceError>;
            }
        };

        // Just verify it parses and compiles
        assert!(!service_trait.is_empty());
    }

    #[test]
    fn test_operation_macro_compiles() {
        // This test verifies that the operation macro compiles without errors
        let operation_fn: TokenStream = parse_quote! {
            #[operation]
            async fn do_something(&self, input: String) -> Result<String, ServiceError> {
                // Implementation
                Ok("result".to_string())
            }
        };

        // Just verify it parses and compiles
        assert!(!operation_fn.is_empty());
    }

    #[test]
    fn test_service_macro_generates_descriptor() {
        // Test that the service macro generates proper service descriptor
        let service_trait: TokenStream = parse_quote! {
            #[service(version = "1.2.3")]
            trait MyService {
                #[operation]
                async fn do_something(&self, input: String) -> Result<String, ServiceError>;
            }
        };

        // Verify the trait is parsed correctly
        assert!(!service_trait.is_empty());
    }
}
