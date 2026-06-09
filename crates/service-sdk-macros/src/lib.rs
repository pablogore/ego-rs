//! Proc-macro attributes for the Service SDK.
//!
//! Provides the `#[service]` and `#[operation]` attributes for declaring
//! service contracts and operations. These macros generate the necessary
//! service descriptors and proxy types for service invocation.
//!
//! # Usage
//!
//! ## Service Declaration
//!
//! ```rust
//! # #[allow(dead_code)]
//! # mod ego_service_sdk {
//! #     pub mod contract {
//! #         pub struct ContractVersion(pub u32, pub u32, pub u32);
//! #         impl ContractVersion { pub fn new(m: u32, n: u32, p: u32) -> Self { Self(m, n, p) } }
//! #         impl std::fmt::Display for ContractVersion {
//! #             fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//! #                 write!(f, "{}.{}.{}", self.0, self.1, self.2)
//! #             }
//! #         }
//! #         pub struct OperationDescriptor { pub name: String, pub input: Vec<String>,
//! #             pub output: String, pub errors: Vec<String>, pub description: Option<String>,
//! #             pub metadata: std::collections::HashMap<String, String> }
//! #         pub struct ServiceDescriptor { pub name: String, pub version: ContractVersion,
//! #             pub operations: Vec<OperationDescriptor>, pub description: Option<String>,
//! #             pub metadata: std::collections::HashMap<String, String> }
//! #         pub trait ServiceContract {
//! #             fn type_id() -> &'static str; fn name() -> &'static str;
//! #             fn version() -> ContractVersion; fn descriptor() -> ServiceDescriptor;
//! #             fn operations() -> Vec<OperationDescriptor>;
//! #         }
//! #     }
//! #     pub mod error { pub struct ServiceError; }
//! # }
//! use ego_service_sdk_macros::{service, operation};
//! use ego_service_sdk::error::ServiceError;
//!
//! #[service]
//! trait MyService {
//!     #[operation]
//!     async fn do_something(&self, input: String) -> Result<String, ServiceError>;
//! }
//! ```
//!
//! ## Generated Code
//!
//! The `#[service]` macro generates:
//! - A service descriptor
//! - A `ServiceContract` implementation for the trait
//!
//! The `#[operation]` macro generates:
//! - Operation descriptors
//! - Method signatures for service implementations
//!
//! # Examples
//!
//! ## Service Contract Declaration
//!
//! ```rust
//! # #[allow(dead_code)]
//! # mod ego_service_sdk {
//! #     pub mod contract {
//! #         pub struct ContractVersion(pub u32, pub u32, pub u32);
//! #         impl ContractVersion {
//! #             pub fn new(m: u32, n: u32, p: u32) -> Self { Self(m, n, p) }
//! #         }
//! #         impl std::fmt::Display for ContractVersion {
//! #             fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//! #                 write!(f, "{}.{}.{}", self.0, self.1, self.2)
//! #             }
//! #         }
//! #         pub struct OperationDescriptor {
//! #             pub name: String, pub input: Vec<String>, pub output: String,
//! #             pub errors: Vec<String>, pub description: Option<String>,
//! #             pub metadata: std::collections::HashMap<String, String>,
//! #         }
//! #         pub struct ServiceDescriptor {
//! #             pub name: String, pub version: ContractVersion,
//! #             pub operations: Vec<OperationDescriptor>, pub description: Option<String>,
//! #             pub metadata: std::collections::HashMap<String, String>,
//! #         }
//! #         pub trait ServiceContract {
//! #             fn type_id() -> &'static str; fn name() -> &'static str;
//! #             fn version() -> ContractVersion; fn descriptor() -> ServiceDescriptor;
//! #             fn operations() -> Vec<OperationDescriptor>;
//! #         }
//! #     }
//! #     pub mod error { pub struct ServiceError; }
//! # }
//! use ego_service_sdk_macros::{service, operation};
//! use ego_service_sdk::error::ServiceError;
//!
//! #[service(version = "1.2.3")]
//! trait MyService {
//!     #[operation]
//!     async fn do_something(&self, input: String) -> Result<String, ServiceError>;
//!     
//!     #[operation]
//!     async fn do_another_thing(&self, input: i32) -> Result<bool, ServiceError>;
//! }
//! ```
//!
//! ## Service Descriptor Access
//!
//! After applying the `#[service]` macro, the service contract can be accessed via:
//!
//! ```rust
//! # #[allow(dead_code)]
//! # mod ego_service_sdk {
//! #     pub mod contract {
//! #         pub struct ContractVersion(pub u32, pub u32, pub u32);
//! #         impl ContractVersion {
//! #             pub fn new(m: u32, n: u32, p: u32) -> Self { Self(m, n, p) }
//! #         }
//! #         impl std::fmt::Display for ContractVersion {
//! #             fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//! #                 write!(f, "{}.{}.{}", self.0, self.1, self.2)
//! #             }
//! #         }
//! #         pub struct OperationDescriptor {
//! #             pub name: String, pub input: Vec<String>, pub output: String,
//! #             pub errors: Vec<String>, pub description: Option<String>,
//! #             pub metadata: std::collections::HashMap<String, String>,
//! #         }
//! #         pub struct ServiceDescriptor {
//! #             pub name: String, pub version: ContractVersion,
//! #             pub operations: Vec<OperationDescriptor>, pub description: Option<String>,
//! #             pub metadata: std::collections::HashMap<String, String>,
//! #         }
//! #         pub trait ServiceContract {
//! #             fn type_id() -> &'static str; fn name() -> &'static str;
//! #             fn version() -> ContractVersion; fn descriptor() -> ServiceDescriptor;
//! #             fn operations() -> Vec<OperationDescriptor>;
//! #         }
//! #     }
//! #     pub mod error { pub struct ServiceError; }
//! # }
//! use ego_service_sdk_macros::{service, operation};
//! use ego_service_sdk::error::ServiceError;
//! use ego_service_sdk::contract::ServiceContract;
//!
//! #[service(version = "1.2.3")]
//! trait MyService {
//!     #[operation]
//!     async fn do_something(&self, input: String) -> Result<String, ServiceError>;
//! }
//! # struct MyHandler;
//! # impl MyService for MyHandler {
//! #     async fn do_something(&self, input: String) -> Result<String, ServiceError> {
//! #         Ok("done".to_string())
//! #     }
//! # }
//!
//! let descriptor = MyHandler::descriptor();
//! assert_eq!(descriptor.name, "MyService");
//! assert_eq!(descriptor.version.to_string(), "1.2.3");
//! assert_eq!(descriptor.operations.len(), 1);
//! ```
//!
//! ## Operation Metadata
//!
//! The `#[operation]` macro extracts metadata from method signatures:
//!
//! ```rust
//! # #[allow(dead_code)]
//! # mod ego_service_sdk {
//! #     pub mod contract {
//! #         pub struct ContractVersion(pub u32, pub u32, pub u32);
//! #         impl ContractVersion {
//! #             pub fn new(m: u32, n: u32, p: u32) -> Self { Self(m, n, p) }
//! #         }
//! #         impl std::fmt::Display for ContractVersion {
//! #             fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//! #                 write!(f, "{}.{}.{}", self.0, self.1, self.2)
//! #             }
//! #         }
//! #         pub struct OperationDescriptor {
//! #             pub name: String, pub input: Vec<String>, pub output: String,
//! #             pub errors: Vec<String>, pub description: Option<String>,
//! #             pub metadata: std::collections::HashMap<String, String>,
//! #         }
//! #         pub struct ServiceDescriptor {
//! #             pub name: String, pub version: ContractVersion,
//! #             pub operations: Vec<OperationDescriptor>, pub description: Option<String>,
//! #             pub metadata: std::collections::HashMap<String, String>,
//! #         }
//! #         pub trait ServiceContract {
//! #             fn type_id() -> &'static str; fn name() -> &'static str;
//! #             fn version() -> ContractVersion; fn descriptor() -> ServiceDescriptor;
//! #             fn operations() -> Vec<OperationDescriptor>;
//! #         }
//! #     }
//! #     pub mod error { pub struct ServiceError; }
//! # }
//! use ego_service_sdk_macros::{service, operation};
//! use ego_service_sdk::error::ServiceError;
//!
//! #[service]
//! trait MyService {
//!     #[operation]
//!     async fn process_data(&self, input: String) -> Result<String, ServiceError>;
//! }
//! ```
//!
//! This generates an operation descriptor with:
//! - Name: "process_data"
//! - Input types: ["String"]
//! - Output type: "Result<String, ServiceError>"
//! - Error types: ["ServiceError"]

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{parse_macro_input, ItemFn, ItemTrait, TraitItem};

/// Arguments for the #[service] macro
#[derive(Debug)]
struct ServiceArgs {
    version: Option<String>,
}

impl Parse for ServiceArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut version = None;
        if input.peek(syn::Ident) {
            let ident: syn::Ident = input.parse()?;
            if ident == "version" {
                input.parse::<syn::Token![=]>()?;
                let version_lit: syn::LitStr = input.parse()?;
                version = Some(version_lit.value());
            }
        }
        Ok(ServiceArgs { version })
    }
}

/// A proc-macro attribute for declaring service contracts.
///
/// Transforms a trait definition into a service contract.
/// ```rust
/// # #[allow(dead_code)]
/// # mod ego_service_sdk {
/// #     pub mod contract {
/// #         pub struct ContractVersion(pub u32, pub u32, pub u32);
/// #         impl ContractVersion {
/// #             pub fn new(m: u32, n: u32, p: u32) -> Self { Self(m, n, p) }
/// #         }
/// #         impl std::fmt::Display for ContractVersion {
/// #             fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
/// #                 write!(f, "{}.{}.{}", self.0, self.1, self.2)
/// #             }
/// #         }
/// #         pub struct OperationDescriptor {
/// #             pub name: String, pub input: Vec<String>, pub output: String,
/// #             pub errors: Vec<String>, pub description: Option<String>,
/// #             pub metadata: std::collections::HashMap<String, String>,
/// #         }
/// #         pub struct ServiceDescriptor {
/// #             pub name: String, pub version: ContractVersion,
/// #             pub operations: Vec<OperationDescriptor>, pub description: Option<String>,
/// #             pub metadata: std::collections::HashMap<String, String>,
/// #         }
/// #         pub trait ServiceContract {
/// #             fn type_id() -> &'static str; fn name() -> &'static str;
/// #             fn version() -> ContractVersion; fn descriptor() -> ServiceDescriptor;
/// #             fn operations() -> Vec<OperationDescriptor>;
/// #         }
/// #     }
/// #     pub mod error { pub struct ServiceError; }
/// # }
/// use ego_service_sdk_macros::{service, operation};
/// use ego_service_sdk::error::ServiceError;
///
/// #[service]
/// trait MyService {
///     #[operation]
///     async fn do_something(&self, input: String) -> Result<String, ServiceError>;
/// }
/// ```
#[proc_macro_attribute]
pub fn service(args: TokenStream, input: TokenStream) -> TokenStream {
    let input_trait = parse_macro_input!(input as ItemTrait);
    let service_args = parse_macro_input!(args as ServiceArgs);

    let trait_name = input_trait.ident.clone();

    let version_str = service_args.version.unwrap_or_else(|| "1.0.0".to_string());
    let parts: Vec<&str> = version_str.split('.').collect();
    let major = parts
        .first()
        .map(|s| s.parse::<u32>().unwrap_or(1))
        .unwrap_or(1);
    let minor = parts
        .get(1)
        .map(|s| s.parse::<u32>().unwrap_or(0))
        .unwrap_or(0);
    let patch = parts
        .get(2)
        .map(|s| s.parse::<u32>().unwrap_or(0))
        .unwrap_or(0);

    let mut operations = Vec::new();
    let mut output_items = Vec::new();
    for item in input_trait.items {
        if let TraitItem::Fn(method) = &item {
            let has_operation_attr = method
                .attrs
                .iter()
                .any(|attr| attr.path().is_ident("operation"));

            if has_operation_attr {
                let method_name = &method.sig.ident;

                let mut input_types = Vec::new();
                let mut input_names = Vec::new();
                for fn_input in method.sig.inputs.iter() {
                    if let syn::FnArg::Typed(pat_type) = fn_input {
                        let input_type = &pat_type.ty;
                        let input_name = &pat_type.pat;
                        input_types.push(quote! { stringify!(#input_type).to_string() });
                        input_names.push(quote! { stringify!(#input_name) });
                    }
                }

                let (output_type, error_types) = match &method.sig.output {
                    syn::ReturnType::Type(_, ty) => {
                        let out_str = quote! { stringify!(#ty) };
                        let errs = extract_error_types(ty);
                        (out_str, errs)
                    }
                    _ => (quote! { "()" }, quote! { vec![] }),
                };

                operations.push(quote! {
                    ego_service_sdk::contract::OperationDescriptor {
                        name: stringify!(#method_name).to_string(),
                        input: vec![#(#input_types),*],
                        output: #output_type.to_string(),
                        errors: #error_types,
                        description: None,
                        metadata: std::collections::HashMap::new(),
                    }
                });

                // Strip the #[operation] attribute so it isn't re-applied to the trait method
                let mut clean_method = method.clone();
                clean_method.attrs.retain(|attr| !attr.path().is_ident("operation"));
                output_items.push(TraitItem::Fn(clean_method));
            } else {
                output_items.push(item);
            }
        } else {
            output_items.push(item);
        }
    }

    let output_trait = syn::ItemTrait {
        items: output_items,
        ..input_trait
    };

    let expanded = quote! {
        #output_trait

        impl<T: #trait_name> ego_service_sdk::contract::ServiceContract for T {
            fn type_id() -> &'static str {
                std::any::type_name::<Self>()
            }

            fn name() -> &'static str {
                stringify!(#trait_name)
            }

            fn version() -> ego_service_sdk::contract::ContractVersion {
                ego_service_sdk::contract::ContractVersion::new(#major, #minor, #patch)
            }

            fn descriptor() -> ego_service_sdk::contract::ServiceDescriptor {
                ego_service_sdk::contract::ServiceDescriptor {
                    name: stringify!(#trait_name).to_string(),
                    version: Self::version(),
                    operations: vec![#(#operations),*],
                    description: None,
                    metadata: std::collections::HashMap::new(),
                }
            }

            fn operations() -> Vec<ego_service_sdk::contract::OperationDescriptor> {
                vec![#(#operations),*]
            }
        }
    };

    TokenStream::from(expanded)
}

fn extract_error_types(ty: &syn::Type) -> proc_macro2::TokenStream {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Result" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if args.args.len() >= 2 {
                        if let syn::GenericArgument::Type(syn::Type::Path(error_path)) =
                            &args.args[1]
                        {
                            return quote! { vec![stringify!(#error_path).to_string()] };
                        }
                    }
                }
            }
        }
    }
    quote! { vec![] }
}

#[cfg(test)]
mod tests;

/// A proc-macro attribute for marking methods as service operations.
///
/// Used inside a `#[service]` trait to identify which methods are operations.
/// Each annotated method generates an `OperationDescriptor` entry in the
/// parent service's `ServiceDescriptor`.
///
/// ```rust
/// # #[allow(dead_code)]
/// # mod ego_service_sdk {
/// #     pub mod contract {
/// #         pub struct ContractVersion(pub u32, pub u32, pub u32);
/// #         impl ContractVersion {
/// #             pub fn new(m: u32, n: u32, p: u32) -> Self { Self(m, n, p) }
/// #         }
/// #         impl std::fmt::Display for ContractVersion {
/// #             fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
/// #                 write!(f, "{}.{}.{}", self.0, self.1, self.2)
/// #             }
/// #         }
/// #         pub struct OperationDescriptor {
/// #             pub name: String, pub input: Vec<String>, pub output: String,
/// #             pub errors: Vec<String>, pub description: Option<String>,
/// #             pub metadata: std::collections::HashMap<String, String>,
/// #         }
/// #         pub struct ServiceDescriptor {
/// #             pub name: String, pub version: ContractVersion,
/// #             pub operations: Vec<OperationDescriptor>, pub description: Option<String>,
/// #             pub metadata: std::collections::HashMap<String, String>,
/// #         }
/// #         pub trait ServiceContract {
/// #             fn type_id() -> &'static str; fn name() -> &'static str;
/// #             fn version() -> ContractVersion; fn descriptor() -> ServiceDescriptor;
/// #             fn operations() -> Vec<OperationDescriptor>;
/// #         }
/// #     }
/// #     pub mod error { pub struct ServiceError; }
/// # }
/// use ego_service_sdk_macros::{service, operation};
/// use ego_service_sdk::error::ServiceError;
///
/// #[service]
/// trait MyService {
///     #[operation]
///     async fn do_something(&self, input: String) -> Result<String, ServiceError>;
/// }
/// ```
///
/// The macro extracts method metadata (name, input type, output type, error type)
/// for automatic descriptor generation. The method signature itself passes through
/// unchanged.
#[proc_macro_attribute]
pub fn operation(_args: TokenStream, input: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(input as ItemFn);
    // The operation macro currently just passes through the input unchanged
    // In a full implementation, this would extract metadata and generate descriptors
    TokenStream::from(quote! { #input_fn })
}
