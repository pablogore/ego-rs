# Tareas: PROD-014C — Reclamo Atómico de Eventos del Lado de Lectura

> Compañero en español (1:1 con identificadores). Fuente canónica: `tasks.md`.
> TDD estricto (AD-10): la suite de contención (Fase 4) se escribe en ROJO contra
> `PostgreSQLReadSideClaimStore`, que aún no existe, antes de los pasos EN VERDE del
> adaptador (Fase 3). Cada aserción de error nombra la variante específica de
> `ClaimError`, nunca `is_err()`. `cargo clippy --workspace -- -W clippy::cognitive-complexity`
> después de cada división.

## Pronóstico de Carga de Revisión

| Campo | Valor |
|-------|-------|
| Líneas cambiadas estimadas | ~1450 en total — PR1 ~320 (puerto + tipos + migración), PR2 ~580 (adaptador + suite de contención con PostgreSQL real), PR3 ~300 (wiring de sesión + scheduler), PR4 ~250 (gate + documentación) |
| Riesgo del presupuesto de 400 líneas | Alto solo para PR2 — una desviación aceptada, con la misma forma que PROD-014B PR2 (D-7 exige la suite real contra PostgreSQL; nunca se separa del adaptador que prueba). PR1, PR3 y PR4 se mantienen en o por debajo del presupuesto |
| ¿Se recomiendan PRs encadenados? | Sí |
| División sugerida | PR 1 (puerto + tipos + migración) → PR 2 (adaptador PostgreSQL + suite de contención real) → PR 3 (wiring de sesión + scheduler) → PR 4 (gate de Producción + documentación) |
| Estrategia de entrega | auto-chain (preflight de sesión) |
| Estrategia de cadena | stacked-to-main — replica la topología de PROD-014A/PROD-014B; cada PR nace del anterior y se fusiona a `develop` en orden |

¿Se necesita decisión antes de aplicar?: No
¿Se recomiendan PRs encadenados?: Sí
Estrategia de cadena: stacked-to-main
Riesgo del presupuesto de 400 líneas: Alto

### Desviación respecto a la división sugerida por design.md (debe justificarse)

El texto de mitigación de R-4 en la propuesta (y la nota de cierre de design.md) sugiere
tres tramos: "puerto + adaptador, luego wiring de sesión/scheduler, luego gate +
documentación". Al estimar la longitud propia de cada tramo, "puerto + adaptador"
combinados —una vez incluidas la migración `016` y la suite de contención obligatoria
contra PostgreSQL real (D-7, IS-7)— llegan a unas 900 líneas, muy por encima incluso de la
desviación de ~500 líneas ya aceptada en PROD-014B PR2. Separar el puerto + migración
(con forma de esquema, fundacional, sin comportamiento de adaptador que probar aún) del
adaptador + suite de contención (el único tramo que D-7 prohíbe recortar) replica
exactamente la propia división PR1/PR2 de PROD-014B, mantiene PR1 cómodamente dentro del
presupuesto, y deja solo a PR2 como desviación aceptada — la misma desviación que
PROD-014B ya estableció como aceptable por esta misma razón (el adaptador nunca se separa
de las pruebas que lo demuestran). Este es un plan de cuatro tramos, no los tres que nombra
la nota de cierre de design.md; el Enfoque y la Semántica Requerida de PROD-014C no se ven
afectados — la desviación es solo de división de entrega.

### Unidades de Trabajo Sugeridas

| Unidad | Objetivo | PR | Comando de prueba enfocado | Arnés de runtime | Frontera de reversión |
|------|------|-----|----------------------|-----------------|-------------------|
| 1 | Puerto `ReadSideClaimStore` + `ClaimId`/`ClaimFence`/`ClaimError` + forwarding de `Arc<T>`; migración `016_create_projection_claims.sql` registrada | PR 1 | `cargo test -p ego-persistence-api read_side::claim`, `cargo test -p ego-persistence migrations` | N/D — solo esquema + forma del trait, aún no hay comportamiento de adaptador que probar | Eliminar `claim.rs`, su re-export, la migración `016` + entrada de registro, revertir el cambio de visibilidad en `reservation.rs`; nada más los referencia todavía |
| 2 | `PostgreSQLReadSideClaimStore` (`try_claim`/`renew`/`release`, fencing, mapeo de errores) + la suite completa de contención con PostgreSQL real (ROJO antes de VERDE, D-7) | PR 2 | `cargo test -p ego-persistence postgres::` (unitario, reuso de `is_fatal`/`claim_error`) + `cargo test -p ego-integration-tests --test read_side_claiming_postgres` | PostgreSQL real vía `isolated_database()`, pools `PgPoolOptions` separados por contendiente, `SettableClock` | Eliminar `read_side_claim.rs`, su re-export, el archivo de la suite de contención; el puerto + migración de PR 1 permanecen válidos y sin uso |
| 3 | Wiring de reclamo/renovación/liberación en `ReadSideSession::execute()`; knob `ProjectionSpec::claims` del scheduler | PR 3 | `cargo test -p ego-domain read_side::session`, `cargo test -p ego-runtime read_side::scheduler` | N/D — dobles con guion, sin pool; solo nivel unitario según AD-10 | Revertir el `ReadSideClaiming`/`with_claiming`/división de `run_batch` en `session.rs` y el knob `claims` en `scheduler.rs`; PR 1–2 permanecen válidos para cualquier wiring manual de host |
| 4 | Gate de fallo cerrado de `Profile::Production`; `AppBuilder::read_side_claims` + guardia de duplicados; documentación de reference-app + `ARCHITECTURE.md` | PR 4 | `cargo test -p ego-service-sdk read_side_claim`, `cargo test -p reference-app` | `examples/reference-app` componiendo bajo `Profile::Production` con un pool real de Postgres | Revertir el slot/validador de `builder.rs`, el método + guardia de duplicados de `app/mod.rs`, la variante de `app/error.rs`, el wiring de reference-app, `ARCHITECTURE.md`; PR 1–3 permanecen funcionalmente completos pero sin uso por el gate |

## Fase 1: Puerto y Tipos (Fundación) — PR 1

- [x] 1.1 ROJO `crates/persistence-api/src/read_side/claim.rs` `#[cfg(test)]`: `Arc<T>` reenvía `is_durable()` al store envuelto, no el `false` por defecto del trait (AD-3 — la mina terrestre de PROD-014A EC-2); igualdad/hash de `ClaimId` por la tripleta completa; igualdad de `ClaimFence` por la tripleta completa.
- [x] 1.2 VERDE: definir `ReadSideClaimStore` (`try_claim`/`renew`/`release`, `is_durable() -> bool { false }` por defecto), `ClaimId { projection_id, tag, tenant }`, `ClaimFence { claim_id, owner_id, fencing_token }`, `ClaimError { StaleOwner, FencingExhausted, Transient, Fatal }` (AD-1, AD-2).
- [x] 1.3 VERDE: `impl<T: ReadSideClaimStore + Send + Sync + ?Sized> ReadSideClaimStore for Arc<T>` reenviando los tres métodos e `is_durable()` explícitamente (AD-3).
- [x] 1.4 VERDE: `crates/persistence-api/src/read_side/mod.rs` — agregar `pub mod claim;` y re-exportar `ReadSideClaimStore`, `ClaimId`, `ClaimFence`, `ClaimError`.
- [x] 1.5 VERDE: `crates/persistence/src/postgres/reservation.rs` — cambiar `token_from_storage` de privado a `pub(crate)` (AD-3); sin cambio de comportamiento, reutilizado sin cambios por la Fase 3.

## Fase 2: Migración — PR 1

- [x] 2.1 Crear `crates/persistence/src/postgres/migrations/016_create_projection_claims.sql`: `projection_claims(projection_id, tag, tenant, owner_id, fencing_token, lease_until, claimed_at)`, `PRIMARY KEY (projection_id, tag, tenant)`, `CHECK (fencing_token > 0)`, sin columna `state`, sin índice sobre `lease_until` (AD-8, D-6). Traza: "Claim Identity Is `(projection_id, tag, tenant)`".
- [x] 2.2 Registrar como constante `include_str!` + una entrada ascendente en `migrations.rs::migrations()`. No se necesita prueba nueva — ejecutar `cargo test -p ego-persistence migrations` para confirmar que las pruebas de registro existentes cubren `016`.

## Fase 3: Adaptador PostgreSQL — PR 2

- [x] 3.1 VERDE `crates/persistence/src/postgres/read_side_claim.rs`: `PostgreSQLReadSideClaimStore { pool: PgPool, clock: Arc<dyn Clock> }`, `Debug` manual (solo el pool), `is_durable() -> true`; `try_claim` como la única sentencia `INSERT … ON CONFLICT (projection_id, tag, tenant) DO UPDATE … WHERE projection_claims.lease_until <= $now RETURNING fencing_token` — sin ventana de verificar-luego-actuar (AD-5).
- [x] 3.2 VERDE: helper privado compartido con forma `mutate_owned` para `renew`/`release`, verificando `(projection_id, tag, tenant, owner_id, fencing_token, lease_until > now)` en un solo `WHERE` por sentencia; `release` fija `lease_until = now`, nunca `DELETE`, manteniendo el token de fencing estrictamente monótono a través de la frontera de liberación (criterios de AD-5).
- [x] 3.3 VERDE: el mapeo `claim_error` reutiliza textualmente el `pub(crate) is_fatal` de PROD-014B para la división `Transient`/`Fatal`, verificando primero el SQLSTATE `22003` (`numeric_value_out_of_range`) → `ClaimError::FencingExhausted`.
- [x] 3.4 VERDE: `crates/persistence/src/postgres/mod.rs` — `pub use read_side_claim::PostgreSQLReadSideClaimStore;`.

## Fase 4: Suite de Contención con PostgreSQL Real — ROJO antes de VERDE de Fase 3 (`integration-tests/tests/infrastructure/read_side_claiming_postgres.rs`) — PR 2

Escrita contra un `PostgreSQLReadSideClaimStore` que aún no existe, según D-7/AD-10; el
fallo de compilación es el estado ROJO esperado. El arnés replica
`takeover_fencing_postgres.rs` / `concurrent_replicas_postgres.rs`: `isolated_database()`
por prueba, pools separados por contendiente, `SettableClock` movido a mano,
`tokio::sync::Barrier`, observadores `AtomicUsize`, aserciones acotadas por `WAIT_LIMIT`,
estado final leído con `sqlx::query_as` crudo, nunca a través del puerto bajo prueba.

- [x] 4.1 ROJO — exclusión SC-1: dos workers, dos pools, dos `OwnerId`, liberados juntos sobre un `(projection_id, tag, tenant)`; exactamente uno obtiene `Some(fence)`, los contadores de fetch/handler del rechazado son ambos 0. Caso de control: los mismos dos workers en dos tenants distintos obtienen ambos un fence y ambos se ejecutan. Traza: "Acquisition Excludes A Concurrent Second Claimant".
- [x] 4.2 ROJO — takeover SC-2: A reclama y nunca libera (la sesión se descarta a mitad de lote); el reloj avanza más allá de `lease_until`; el `try_claim` de B devuelve `Some`, `fencing_token` estrictamente mayor, el `owner_id` de la fila es el de B. Traza: "An Expired Lease Enables Takeover Without Operator Action".
- [x] 4.3 ROJO — rechazo de dueño obsoleto SC-3: tras el takeover de B, el `renew`/`release` de A son ambos `Err(StaleOwner)`, la fila sigue con el owner y token de B sin cambios; más una prueba de aislamiento de token — el `owner_id` de B emparejado con el `fencing_token` obsoleto de A también es rechazado, de modo que el rechazo nunca es atribuible solo a `owner_id`. Traza: "Takeover Fences Out The Stale Owner".
- [x] 4.4 ROJO — la renovación previene el takeover: A renueva antes de expirar; el intento concurrente de `try_claim` de B durante la lease renovada es rechazado. Traza: "A Valid Claim May Be Renewed To Extend Processing".
- [x] 4.5 ROJO — orden SC-5: un worker mantiene el reclamo a través de un lote de al menos tres eventos; la porción recibida por el handler se afirma estrictamente ascendente por `event_version`. Traza: "Claiming Preserves Existing Per-Stream Ordering".
- [x] 4.6 ROJO — reclamo inmediato tras liberación: un worker libera normalmente; un segundo `try_claim` inmediatamente después tiene éxito sin esperar a que expire la lease. Traza: "Normal Release Makes the Stream Immediately Reclaimable".
- [x] 4.7 Verificaciones de mutación, registradas en la documentación del módulo de la suite en lugar de asumidas: eliminar `AND projection_claims.lease_until <= $6` del `WHERE` de `try_claim` debe hacer fallar 4.1 con ambos workers reclamando; eliminar `AND fencing_token = $6` del `WHERE` de fence compartido debe hacer fallar la prueba de token de 4.3. Confirmado a mano una vez, documentado, nunca dejado en el diff entregado como un estado roto.
- [x] 4.8 VERDE: confirmar que 4.1–4.6 pasan una vez que el adaptador de la Fase 3 aterriza.

## Fase 5: Wiring de Sesión — PR 3

- [ ] 5.1 ROJO `crates/domain/src/read_side/session.rs` `#[cfg(test)]`, dobles con guion, sin pool: un `try_claim` rechazado (`Ok(None)`) ⇒ `fetch` nunca se llama, el handler nunca se invoca, `execute()` devuelve `Ok(None)` (IS-4, AD-4).
- [ ] 5.2 ROJO: `renew` devolviendo `StaleOwner` ⇒ ni `mark_seen` ni `write_offset`, el error se propaga como `ProjectionError::transient` nombrando las escrituras retenidas (AD-6).
- [ ] 5.3 ROJO: `release` se llama en el camino de éxito, en ambos caminos de retorno temprano vacío (`events.is_empty()`, `unique_events.is_empty()`), y en el camino de error del handler.
- [ ] 5.4 VERDE: agregar `ReadSideClaiming { store, owner, clock, lease }` y `with_claiming(...)` como un knob opcional — cada sitio de llamada existente a `ReadSideSession::new` compila sin cambios; dividir `execute()` en la puerta `try_claim` + el cuerpo extraído `run_batch`, con `release` llamado incondicionalmente en cada camino de salida (AD-6).
- [ ] 5.5 VERDE: insertar la llamada `renew` entre `handler.handle()` y el bucle de commit dentro de `run_batch`; mapear `StaleOwner` a `ProjectionError::transient` con la redacción exacta de AD-6, otros errores a `ProjectionError::transient(format!("claim renew failed: {other}"))`.
- [ ] 5.6 Rustdoc en `ReadSideClaiming::owner`: declarar que la unicidad de `OwnerId` por instancia de proceso es obligación del host, que el puerto no puede verificarla, y nombrar la consecuencia de violarla (Pregunta Abierta documentada — no una brecha de código por cerrar).
- [ ] 5.7 `crates/domain/src/read_side/mod.rs`: re-exportar los tipos de `claim` en la forma de ruta existente del módulo, replicando `offset`/`dedup`.

## Fase 6: Wiring del Scheduler — PR 3

- [ ] 6.1 ROJO `crates/runtime/src/read_side/scheduler.rs`: `ProjectionSpec::claims(claiming)` fija el knob, ausente por defecto (replica `reporter`/`interval`/`on_error`); `TagSchedulerImpl::start_projection` lo adjunta a cada sesión que construye (AD-7).
- [ ] 6.2 VERDE: agregar `pub fn claims(mut self, claiming: ReadSideClaiming) -> Self` a `ProjectionSpec`; mover `spec.claiming` a `TagSchedulerImpl` dentro de `spawn`; `start_projection` lee `self.claiming` y llama a `.with_claiming(...)` cuando está presente. La firma pública de `TagScheduler::start_projection` permanece sin cambios — ningún implementador externo se rompe.
- [ ] 6.3 Confirmar que `start_projection` permanece como el bucle for secuencial de hoy — sin concurrencia entre tags agregada (D-12, OOS-5); sin estado de reclamo entre ticks, sin caché de fence en memoria.

## Fase 7: Gate de Producción — PR 4

- [ ] 7.1 ROJO `crates/service-sdk/src/runtime/builder.rs`: matriz {Dev, Production} × {sin progreso / sin claim store, progreso registrado / sin claim store, progreso registrado / claim store volátil, progreso registrado / claim store durable}; Production + cero progreso registrado + sin claim store ⇒ `Ok` (la forma de retorno temprano dentro de la función, PROD-014A EC-1); `build()`/`try_build()` concuerdan (SC-4).
- [ ] 7.2 VERDE: agregar el campo `read_side_claims: Option<Arc<dyn ReadSideClaimStore + Send + Sync>>`; `validate_read_side_claim_profile` devuelve `Ok(())` temprano cuando `self.read_side_progress.is_empty()`, si no, llama a `require_durably_configured(self.profile, self.read_side_claims.as_ref().is_some_and(|c| c.is_durable()), "durable read-side claim store (ReadSideClaimStore)", "AppBuilder::read_side_claims(store) (or RuntimeBuilder::with_read_side_claim_store(..))")` textualmente; se llama desde `validate_persistence_profile` después de los dos validadores existentes (AD-9).
- [ ] 7.3 ROJO `crates/service-sdk/src/app/error.rs`: `CompositionError::DuplicateReadSideClaimStore` cuyo mensaje nombra la llamada infractora, sugiere que no hay API de reemplazo (replica `DuplicateReadSideProgress`, PROD-014A 3.1).
- [ ] 7.4 VERDE: agregar la variante; `crates/service-sdk/src/app/mod.rs` — `AppBuilder::read_side_claims(store)` con una guardia de duplicados de fallo cerrado; `RuntimeBuilder` permanece de última-escritura-gana (replica la división de `effect_store`, criterio d de AD-9).

## Fase 8: Reference-App y Documentación — PR 4

- [ ] 8.1 `examples/reference-app/src/read_side/mod.rs:118-126`: retirar el comentario "PROD-014C es la brecha no aplicada"; conectar (o documentar explícitamente la ausencia de) un registro de claim store, reflejando el mecanismo ahora aplicado.
- [ ] 8.2 `ARCHITECTURE.md:211-219`: reemplazar el lenguaje de escritor único no aplicado con la descripción de reclamo aplicado, nombrando `read-side-event-claiming`.
- [ ] 8.3 Confirmar que `openspec/changes/prod-014c-atomic-read-side-event-claiming/specs/{read-side-event-claiming,read-side}/spec.md` (ya redactados) son los deltas exactos que `sdd-archive` fusiona — sin edición adicional en esta tarea.
- [ ] 8.4 Gate de grep (SC-6, R-1): confirmar que ningún archivo tocado por este cambio afirma la garantía propia de esta capacidad como "exactamente una vez" — una coincidencia dentro de la redacción de no-objetivo de OOS-2/D-8 es esperada; una coincidencia que afirme un logro de exactamente-una-vez no lo es y debe corregirse antes de fusionar.

## Fase 9: Verificación Final — PR 4

- [ ] 9.1 `cargo test --workspace` cero fallos nuevos (SC-5); `cargo clippy --workspace -- -D warnings` limpio; confirmar que ninguna función tocada excede complejidad cognitiva 10.
- [ ] 9.2 Re-ejecutar `cargo test -p ego-integration-tests --test read_side_claiming_postgres`; confirmar que 4.1–4.6 están todas en VERDE, y que las verificaciones de mutación de 4.7 están documentadas pero nunca se dejan rotas en el diff entregado.
- [ ] 9.3 Confirmación por lectura de diff (sin cambio de código): cada sentencia SQL en `read_side_claim.rs` y `016_create_projection_claims.sql` se vincula vía `$N`, cero interpolación de cadenas (Matriz de Amenazas — Reglas 1/2 cerradas por construcción).

## Auditoría de Trazabilidad

Todos los requisitos AGREGADOS (`read-side-event-claiming`) y MODIFICADOS (`read-side`)
mapeados a al menos una tarea que los cubre:

| Requisito | Capacidad | Tarea(s) que cubre(n) |
|---|---|---|
| Claim Identity Is `(projection_id, tag, tenant)` | `read-side-event-claiming` | 1.2, 2.1, 4.1 |
| Acquisition Excludes A Concurrent Second Claimant | `read-side-event-claiming` | 3.1, 4.1 |
| A Valid Claim May Be Renewed To Extend Processing | `read-side-event-claiming` | 3.2, 4.4 |
| An Expired Lease Enables Takeover Without Operator Action | `read-side-event-claiming` | 3.1, 4.2 |
| Takeover Fences Out The Stale Owner | `read-side-event-claiming` | 3.2, 5.2, 4.3 |
| Normal Release Makes the Stream Immediately Reclaimable | `read-side-event-claiming` | 3.2, 4.6 |
| Claiming Preserves Existing Per-Stream Ordering | `read-side-event-claiming` | 5.4, 4.5 |
| Expiry Is Evaluated Consistently, Never Against An Individual Worker's Own Clock | `read-side-event-claiming` | 3.1 (`Clock` inyectado), 5.4, 5.5 |
| `Profile::Production` Fails Closed Without A Durable Claim Mechanism | `read-side-event-claiming` | 7.1, 7.2 |
| This Capability Bounds Handler-Execution Count, Never External Side-Effect Count | `read-side-event-claiming` | 5.6, 8.4 |
| Prevention of Double Handler Execution Is Enforced By Atomic Claiming Across Replicas | `read-side` (MODIFICADO) | 5.4, 5.5, 4.1 |
| The Concurrency Gap Named In PROD-014B Is Discharged By Atomic Claiming | `read-side` (MODIFICADO) | 8.1, 8.2 |

**Verificación cruzada de frontera de alcance contra el Fuera de Alcance de la propuesta y
las referencias OOS del diseño — cero hallazgos.** Ninguna tarea de esta lista agrega:
consenso distribuido, elección de líder, o un broker (OOS-1); una garantía de
exactamente-una-vez para efectos externos (OOS-2 — 8.4 aplica el gate de redacción en su
lugar); retry/backoff para errores `Transient` (OOS-3 — sin tocar); atomicidad
cross-tabla entre dedup y offset (OOS-4 — 5.4/5.5 nombran la ventana residual, nunca la
cierran); concurrencia entre tags dentro del mismo proceso (OOS-5 — 6.3 confirma que
`start_projection` permanece secuencial); o cualquier backend distinto de PostgreSQL
(OOS-6 — cada tarea de adaptador apunta a `crates/persistence/src/postgres/`). Las tres
Preguntas Abiertas en design.md (ventana residual de fence/escritura, unicidad de
`OwnerId` por proceso, proyecciones fuera de la raíz de composición sin gobernar) están
documentadas como limitaciones aceptadas (5.6, la redacción propia de AD-6) — ninguna
tarea intenta cerrarlas.
