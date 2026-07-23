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

    // -------------------------------------------------------------------
    // CORE-028 Stage 2B (task 2.1) — `ServiceArgs` gains an `impl_of`
    // field. These reference `crate::ServiceArgs::impl_of` and
    // `crate::tag_path_from_impl_of`, neither of which exist yet — RED.
    // -------------------------------------------------------------------

    #[test]
    fn service_args_parses_bare_ident_impl_of() {
        let args: crate::ServiceArgs = parse_quote! { impl_of = MyTrait };
        assert!(args.version.is_none());
        let path = args.impl_of.expect("impl_of must parse");
        assert_eq!(path.segments.len(), 1);
        assert_eq!(path.segments.last().unwrap().ident, "MyTrait");
    }

    #[test]
    fn service_args_parses_path_qualified_impl_of() {
        let args: crate::ServiceArgs = parse_quote! { impl_of = crate::foo::MyTrait };
        let path = args.impl_of.expect("impl_of must parse");
        assert_eq!(path.segments.len(), 3);
        assert_eq!(path.segments[0].ident, "crate");
        assert_eq!(path.segments[1].ident, "foo");
        assert_eq!(path.segments.last().unwrap().ident, "MyTrait");
    }

    #[test]
    fn service_args_parses_version_and_impl_of_combined() {
        let args: crate::ServiceArgs = parse_quote! { version = "1.2.3", impl_of = MyTrait };
        assert_eq!(args.version.as_deref(), Some("1.2.3"));
        assert_eq!(
            args.impl_of
                .expect("impl_of must parse")
                .segments
                .last()
                .unwrap()
                .ident,
            "MyTrait"
        );
    }

    #[test]
    fn service_args_parses_impl_of_before_version() {
        let args: crate::ServiceArgs = parse_quote! { impl_of = MyTrait, version = "2.0.0" };
        assert_eq!(args.version.as_deref(), Some("2.0.0"));
        assert_eq!(
            args.impl_of
                .expect("impl_of must parse")
                .segments
                .last()
                .unwrap()
                .ident,
            "MyTrait"
        );
    }

    // Duplicate keys must be rejected loudly rather than silently last-winning,
    // mirroring `#[authorize]`'s duplicate-argument guards.

    #[test]
    fn service_args_rejects_duplicate_version() {
        let err = syn::parse_str::<crate::ServiceArgs>("version = \"1.0.0\", version = \"2.0.0\"")
            .expect_err("expected Err for duplicate version");
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate 'version' argument"),
            "duplicate version message mismatch: {msg}"
        );
    }

    #[test]
    fn service_args_rejects_duplicate_impl_of() {
        let err = syn::parse_str::<crate::ServiceArgs>("impl_of = Foo, impl_of = Bar")
            .expect_err("expected Err for duplicate impl_of");
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate 'impl_of' argument"),
            "duplicate impl_of message mismatch: {msg}"
        );
    }

    // Tag ident derives from the final path segment (`MyTraitTag`) while the
    // trait reference itself preserves the full module path — the two
    // things `impl_of`'s codegen must get right independently (task 2.1).
    #[test]
    fn tag_path_derives_final_segment_ident_and_preserves_module_path() {
        let path: syn::Path = parse_quote! { crate::foo::MyTrait };
        let tag_path = crate::tag_path_from_impl_of(&path);

        assert_eq!(tag_path.segments.last().unwrap().ident, "MyTraitTag");
        assert_eq!(
            tag_path.segments.len(),
            3,
            "module path segments must be preserved"
        );
        assert_eq!(tag_path.segments[0].ident, "crate");
        assert_eq!(tag_path.segments[1].ident, "foo");

        // The original path (used for the `dyn Trait` reference) is untouched.
        assert_eq!(path.segments.last().unwrap().ident, "MyTrait");
    }

    #[test]
    fn tag_path_for_bare_ident_impl_of() {
        let path: syn::Path = parse_quote! { MyTrait };
        let tag_path = crate::tag_path_from_impl_of(&path);
        assert_eq!(tag_path.segments.len(), 1);
        assert_eq!(tag_path.segments.last().unwrap().ident, "MyTraitTag");
    }

    // -------------------------------------------------------------------
    // CORE-028 Stage 2C DX follow-up — `#[service]` structs must
    // auto-recognize an `EntityRuntimeRef<E>` field as an Entity DI
    // dependency, exactly as they already do for `ProjectionRef<P>`.
    // -------------------------------------------------------------------

    #[test]
    fn classify_field_type_recognizes_entity_runtime_ref() {
        let ty: syn::Type = parse_quote! { EntityRuntimeRef<SomeEntity> };
        let dep_key = crate::classify_field_type(&ty)
            .expect("EntityRuntimeRef<E> must classify as a DI dependency");
        let rendered = dep_key.to_string();
        assert!(
            rendered.contains("DepKey") && rendered.contains("Entity"),
            "expected a DepKey::Entity, got: {rendered}"
        );
    }

    #[test]
    fn classify_field_init_resolves_entity_runtime_ref() {
        let ty: syn::Type = parse_quote! { EntityRuntimeRef<SomeEntity> };
        let init = crate::classify_field_init(&ty)
            .expect("EntityRuntimeRef<E> must produce a DI init expression");
        let rendered = init.to_string();
        assert!(
            rendered.contains("resolve_entity"),
            "expected a resolve_entity init, got: {rendered}"
        );
    }
}
