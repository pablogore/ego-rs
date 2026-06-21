use std::collections::HashMap;

/// A service descriptor.
#[derive(Debug, Clone)]
pub struct ServiceDescriptor {
    /// The name of the service.
    pub name: String,
    /// The version of the service.
    pub version: crate::contract::ContractVersion,
    /// The operations available on the service.
    pub operations: Vec<OperationDescriptor>,
    /// The description of the service.
    pub description: Option<String>,
    /// The metadata of the service.
    pub metadata: HashMap<String, String>,
}

/// An operation descriptor.
#[derive(Debug, Clone)]
pub struct OperationDescriptor {
    /// The name of the operation.
    pub name: String,
    /// The input types of the operation.
    pub input: Vec<String>,
    /// The output type of the operation.
    pub output: String,
    /// The error types of the operation.
    pub errors: Vec<String>,
    /// The description of the operation.
    pub description: Option<String>,
    /// The metadata of the operation.
    pub metadata: HashMap<String, String>,
    /// Whether the operation is idempotent.
    pub idempotent: bool,
    /// Whether the operation mutates state.
    pub mutating: bool,
}

impl Default for OperationDescriptor {
    fn default() -> Self {
        Self {
            name: String::new(),
            input: Vec::new(),
            output: String::new(),
            errors: Vec::new(),
            description: None,
            metadata: HashMap::new(),
            idempotent: false,
            mutating: true,
        }
    }
}

/// A contract descriptor.
#[derive(Debug, Clone)]
pub struct ContractDescriptor {
    /// The name of the contract.
    pub name: String,
    /// The version of the contract.
    pub version: crate::contract::ContractVersion,
    /// The fields of the contract.
    pub fields: Vec<FieldDescriptor>,
    /// The description of the contract.
    pub description: Option<String>,
    /// The metadata of the contract.
    pub metadata: HashMap<String, String>,
}

/// A field descriptor.
#[derive(Debug, Clone)]
pub struct FieldDescriptor {
    /// The name of the field.
    pub name: String,
    /// The type of the field.
    pub field_type: String,
    /// The description of the field.
    pub description: Option<String>,
    /// The metadata of the field.
    pub metadata: HashMap<String, String>,
    /// Whether the field is required.
    pub required: bool,
}

impl Default for FieldDescriptor {
    fn default() -> Self {
        Self {
            name: String::new(),
            field_type: String::new(),
            description: None,
            metadata: HashMap::new(),
            required: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_descriptor_has_idempotency_flag() {
        let op = OperationDescriptor {
            name: "get_order".to_string(),
            input: vec!["OrderId".to_string()],
            output: "Order".to_string(),
            errors: vec![],
            description: None,
            metadata: HashMap::new(),
            idempotent: true,
            mutating: false,
        };
        assert!(op.idempotent);
        assert!(!op.mutating);

        // Default: idempotent=false, mutating=true
        let default_op = OperationDescriptor::default();
        assert!(!default_op.idempotent);
        assert!(default_op.mutating);
    }

    #[test]
    fn field_descriptor_has_required_flag() {
        let field = FieldDescriptor {
            name: "order_id".to_string(),
            field_type: "String".to_string(),
            description: None,
            metadata: HashMap::new(),
            required: false,
        };
        assert!(!field.required);

        // Default: required=true
        let default_field = FieldDescriptor::default();
        assert!(default_field.required);
    }
}
