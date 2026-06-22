//! Principal identity model — [`Principal`], [`PrincipalKind`], [`SubjectId`], [`Role`], [`Claim`].

pub mod principal;
pub mod subject_id;

pub use principal::{Attribute, Claim, Principal, PrincipalKind, Role};
pub use subject_id::SubjectId;
