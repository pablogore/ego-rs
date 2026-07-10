# Delta for testkit

Scope: CORE-025 F-06, F-07. `FixtureBuilder`/`ServiceTestFixture` gain trait-proxy registration and resolution that route through the identical production `RuntimeBuilder::with_service`/`Runtime::resolve` path added to service-sdk by this change — matching the "same canonical path" proof `FixtureBuilder::build()` already provides for the `Injectable` DI path.

## ADDED Requirements

### Requirement: TestKit Trait-Proxy Registration and Resolution Use the Canonical Production Path

`FixtureBuilder` MUST provide `with_service::<Tag>(self, svc: Arc<Tag::Service>) -> Result<Self, RegistryError>` and `ServiceTestFixture` MUST provide `resolve::<Tag>(&self) -> Result<Tag::Proxy, RuntimeError>`. Both MUST be pass-throughs to the identical `RuntimeBuilder::with_service`/`Runtime::resolve` this change adds to `service-sdk` — TestKit MUST NOT assemble a parallel or bespoke trait-proxy construction (no separate `InterceptorChain`/`Weak` wiring inside TestKit). This mirrors the existing, verified proof pattern for the `Injectable` DI path, where `FixtureBuilder::build()` calls the same `RuntimeInner::new_with_logger` constructor `RuntimeBuilder::build()` calls.

#### Scenario: FixtureBuilder registration reaches the same registry the production builder would populate
- GIVEN `FixtureBuilder::new().with_service::<HelloServiceTag>(Arc::new(HelloServiceImpl))?`
- WHEN the fixture builds its internal `Runtime`
- THEN the registered service is stored under the same `(Tag, version)` keying `RuntimeBuilder::with_service` would use — no separate TestKit-only registry exists

#### Scenario: ServiceTestFixture resolution yields the same generated proxy as production resolve()
- GIVEN a `ServiceTestFixture` built with `.with_service::<HelloServiceTag>(..)` registered
- WHEN `fixture.resolve::<HelloServiceTag>()` is called
- THEN `Ok(HelloServiceRef)` is returned, invoking the operation runs the identical guard order (authorize → tenant → interceptors → body) a production `rt.resolve::<HelloServiceTag>()` call would produce

#### Scenario: Resolving an unregistered tag from a test fixture fails the same way production does
- GIVEN a `ServiceTestFixture` with no registration for `OtherServiceTag`
- WHEN `fixture.resolve::<OtherServiceTag>()` is called
- THEN `Err(RuntimeError::ServiceNotFound)` is returned — the same error variant and condition `Runtime::resolve` returns
