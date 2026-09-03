# Tareas: STOOLAP-S1 — Adaptador `Repository` de Stoolap de Primera Clase

> Compañero en español. Fuente canónica de la verdad: `tasks.md` (identificadores 1:1).
> TDD estricto (`openspec/config.yaml` → `apply.tdd: true`): el RED de cada rebanada es, o bien
> un fallo de compilación que nombra una ruta que aún no existe, o bien una prueba unitaria nueva
> que nombra una función que aún no existe (design AD-11). Ninguna tarea de este cambio es una
> prueba de caracterización — cada RED aquí verifica comportamiento genuinamente nuevo; nada aquí
> redocumenta código que ya pasaba. El orden de rebanadas es el obligatorio de design AD-11: S1
> (harness) → S2 (crate) → S3 (tercer sujeto), cada una compilando el workspace completo antes de
> que empiece la siguiente (propiedad de rollback a mitad de vuelo, proposal Rollback Plan).
>
> **Nota OQ-1**: este diseño implementa la semántica documentada de agregado nuevo (EC-1) y
> excluye ese escenario del harness compartido (spec R6). Ninguna tarea reconcilia
> `PostgreSQLRepository`; eso es F-5, archivado por separado (NG-9/R11).
> **Nota OQ-2**: resuelta en `spec.md` R12 — solo garantía de un único proceso propietario;
> ninguna tarea reclama seguridad multi-proceso.

## Pronóstico de Carga de Revisión

Líneas base medidas (no estimadas): `crates/persistence/src/postgres/repository.rs` 214 líneas ·
`integration-tests/tests/infrastructure/repository_tenant_scoping_postgres.rs` 213 líneas (las
3 pruebas que el harness generaliza a 11 escenarios) · `crates/testkit/src/event_store.rs` ~268
líneas (la plantilla de harness existente más cercana, design AD-8 criterio 1).

| Campo | Valor |
|-------|-------|
| Líneas cambiadas estimadas | ~815–995 en total — S1 ~340–400 (harness + 2 puntos de llamada + comentario de doc), S2 ~430–530 (crate + esquema + `save`/`load`/`delete` + 7 pruebas unitarias colocadas — la rebanada más grande), S3 ~45–65 (dev-deps + 1 archivo de prueba) |
| Riesgo del presupuesto de 400 líneas | Alto para el total combinado y para S2 por sí sola; S1 en el límite Alto; S3 Bajo |
| Se recomiendan PRs encadenados | Sí |
| División sugerida | PR 1 (S1 — harness + corridas Memory y PostgreSQL) → PR 2 (S2 — crate, esquema, CAS) → PR 3 (S3 — corrida del harness contra Stoolap) |
| Estrategia de entrega | ask-on-risk |
| Estrategia de encadenamiento | pendiente — se necesita decisión del usuario (se recomienda stacked-to-main, coherente con el orden obligatorio S1→S2→S3 de AD-11 y el rollback a mitad de vuelo por rebanada) |

Decisión necesaria antes de aplicar: Sí
PRs encadenados recomendados: Sí
Estrategia de encadenamiento: pendiente
Riesgo del presupuesto de 400 líneas: Alto

**Costura de reserva nombrada (design AD-11 criterio 3)**: si S2 excede el presupuesto en la
práctica, dividirla en S2a (`new` + esquema + `load`/`delete`) y S2b (algoritmo CAS de `save`),
moviendo la prueba de duplicado systemwide de R3 a S2b. Registrado como decisión, no como
improvisación.

### Unidades de Trabajo Sugeridas

| Unidad | Objetivo | PR | Rama base | Comando de prueba enfocado | Arnés en tiempo de ejecución | Límite de rollback |
|------|------|-----|----------------|----------------------|-----------------|-------------------|
| 1 | S1 — harness compartido, verde contra Memory y PostgreSQL antes de que exista código de Stoolap (RK-4) | PR 1 | `develop` | `cargo test -p ego-testkit --test repository_conformance_memory` | `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` (PostgreSQL real) | Eliminar `repository_conformance.rs`, su par `mod`/`pub use`, y ambos archivos de prueba de punto de llamada; `ego-testkit`/`integration-tests` siguen compilando |
| 2 | S2 — crate `ego-persistence-stoolap`: esquema, `save`/`load`/`delete`, mapeo de errores | PR 2 | PR 1 | `cargo test -p ego-persistence-stoolap` | `cargo test -p ego-persistence-stoolap -- race_between_two_transactions_is_a_conflict` (carrera real de dos transacciones contra un archivo Stoolap temporal) | Eliminar `crates/persistence-stoolap/`, quitar el miembro del workspace y la entrada de `layers.toml`; PR 1 sigue siendo válido |
| 3 | S3 — Stoolap se convierte en el tercer sujeto del harness | PR 3 | PR 2 | `cargo test -p ego-persistence-stoolap --test repository_conformance` | Mismo comando — base de datos Stoolap embebida real en una ruta `tempfile::TempDir` nueva | Eliminar el archivo de prueba y las dos líneas de dev-dependency; PR 1–2 permanecen válidos |

## Fase 1: RED — Puntos de Llamada del Harness Antes de que Exista — S1 — PR 1

- [x] 1.1 Crear `crates/testkit/tests/repository_conformance_memory.rs`: construir `InMemoryRepository<ConformanceAggregate, _>`, llamar a `ego_testkit::assert_repository_conformance`. Falla al compilar — ningún símbolo existe todavía (AD-11).
- [x] 1.2 Crear `integration-tests/tests/infrastructure/repository_conformance_postgres.rs`: construir `PostgreSQLRepository<ConformanceAggregate, _>` contra `isolated_database()`, llamar a la misma función del harness; registrar una línea `mod repository_conformance_postgres;` en `integration-tests/tests/infrastructure.rs`. Falla al compilar — misma razón.

## Fase 2: GREEN — El Harness Compartido y la Corrida contra Memory — S1 — PR 1

- [x] 2.1 Crear `crates/testkit/src/repository_conformance.rs`: `ConformanceAggregate`, `conformance_aggregate(value: &str)`, y `assert_repository_conformance<R: Repository<ConformanceAggregate> + ?Sized>(repository: &mut R)` implementando los 11 escenarios (tabla design AD-8) — spec R1, R2, R3, R4, R5, R7, R8. El comentario de documentación nombra las 4 exclusiones deliberadas: `expected_version` distinto de cero en agregado nuevo (EC-1, spec R6), durabilidad, concurrencia, forma del payload.
- [x] 2.2 Exportar desde `crates/testkit/src/lib.rs`: `pub mod repository_conformance;` más `pub use` de los tres elementos públicos, junto a los tres harnesses de conformidad existentes.
- [x] 2.3 Confirmar que 1.1 ahora compila y pasa — los 11 escenarios en verde contra `InMemoryRepository` (spec R2, sujeto 1).

## Fase 3: RED+GREEN — La Corrida contra PostgreSQL — S1 — PR 1

- [x] 3.1 Confirmar que 1.2 ahora compila y pasa contra PostgreSQL real — los 11 escenarios en verde (spec R2, sujeto 2).
- [x] 3.2 Confirmar que `repository_tenant_scoping_postgres.rs` sigue pasando sin modificar — sus 3 pruebas son ahora un subconjunto del harness compartido (design AD-9 criterio 4).

## Fase 4: Verificación — S1 — PR 1

- [x] 4.1 `cargo test -p ego-testkit` pasa con la corrida de conformidad contra Memory en verde.
- [x] 4.2 `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` pasa con la corrida de conformidad contra PostgreSQL en verde.
- [x] 4.3 Confirmar cero aristas de dependencia nuevas: `crates/persistence-memory/**` intacto (EC-2); `crates/testkit/Cargo.toml` e `integration-tests/Cargo.toml` sin cambios (AD-9 criterios 1–2).
- [x] 4.4 Confirmar que el comentario de documentación del harness enuncia las 4 exclusiones completas (AD-8 criterio 5) y que el escenario de agregado nuevo con versión distinta de cero no aparece en ninguna parte de la lista de escenarios (spec R6, segundo escenario).

## Fase 5: Fundación — Esqueleto del Crate y Puerta de Capas — S2 — PR 2

- [x] 5.1 Crear `crates/persistence-stoolap/Cargo.toml` (paquete `ego-persistence-stoolap`): dependencias normales `ego-persistence-api` (path), `stoolap = "0.4"`, `serde = "1"`, `serde_json = "1"` — exactamente el conjunto D-3/AD-1, sin dev-dependencies todavía (design AD-11 difiere `ego-testkit`/`tempfile` a S3).
- [x] 5.2 Crear `src/lib.rs`: doc de crate, `pub mod persistence;`, `pub use persistence::repository::StoolapRepository;` (AD-2). Sin `#![deny(missing_docs)]`.
- [x] 5.3 Crear `src/persistence/mod.rs`: `pub mod repository;`.
- [x] 5.4 Añadir la entrada de `layers.toml` `"ego-persistence-stoolap" = "infrastructure"`. No abrir `xtask/src/layers.rs` (D-2, AD-1).
- [x] 5.5 Añadir `"crates/persistence-stoolap",` a los miembros del workspace en el `Cargo.toml` raíz.

## Fase 6: RED — Pruebas Unitarias de DSN y del Centinela de Tenant — S2 — PR 2

- [x] 6.1 Escribir la prueba unitaria fallida `dsn_carries_full_sync`: `dsn_for(Path::new("/tmp/x")) == "file:///tmp/x?sync=full"`. Falla — `dsn_for` aún no existe (AD-4; matriz de amenazas "Durabilidad").
- [x] 6.2 Escribir la prueba unitaria fallida `encode_tenant_maps_only_the_absent_scope_to_the_sentinel`: `encode_tenant(None) == ""`, `encode_tenant(Some("t")) == "t"`. Falla — `encode_tenant`/`SYSTEMWIDE_SCOPE` aún no existen (AD-3; matriz de amenazas "Aislamiento de tenant"/"Fuga del centinela").

## Fase 7: GREEN — Esquema, DSN, Centinela de Tenant — S2 — PR 2

- [x] 7.1 Añadir la constante DDL `CREATE_AGGREGATES_TABLE` (sección Schema): `tenant_id`, `aggregate_id`, `version`, `payload`, `UNIQUE (tenant_id, aggregate_id)`, sin `PRIMARY KEY` en ningún lugar (EC-3).
- [x] 7.2 Implementar `SYSTEMWIDE_SCOPE` y `encode_tenant()` — pone en verde 6.2 (AD-3).
- [x] 7.3 Implementar `dsn_for()` — pone en verde 6.1 (AD-4).
- [x] 7.4 Implementar `struct StoolapRepository<A, F>`, su impl de `Debug` (imprime `db.dsn()`, no el handle), y `pub fn new(path: &Path, deserialize: F) -> Result<Self, PersistenceError>` que abre vía `dsn_for` y ejecuta el DDL (AD-4; la divergencia del constructor falible de OQ-3, registrada y no oculta).

## Fase 8: RED — Pruebas Unitarias de Durabilidad y CAS — S2 — PR 2

- [x] 8.1 Escribir la prueba unitaria fallida `an_opened_repository_requested_full_sync`: un accesor delgado `dsn()` sobre `Database::dsn()` igual a `dsn_for(path)`. Falla — el accesor aún no existe (EC-6, spec R5 primera mitad).
- [x] 8.2 Escribir la prueba unitaria fallida `a_committed_save_survives_close_and_reopen` (ruta bajo `std::env::temp_dir()` con sufijo único por prueba — sin dependencia `tempfile` hasta S3). Falla — `save`/`load` no implementados (spec R9).
- [x] 8.3 Escribir la prueba unitaria fallida `two_systemwide_saves_leave_exactly_one_row`: guardar el mismo `aggregate_id` dos veces bajo `None`, verificar una sola fila y `version == 2`. Falla — `save` no implementado (spec R3; el fallo que una columna nulable habría permitido).
- [x] 8.4 Escribir la prueba unitaria fallida `a_stale_expected_version_is_a_conflict`. Falla — `save` no implementado (spec R5).
- [x] 8.5 Escribir la prueba unitaria fallida `race_between_two_transactions_is_a_conflict`: dos transacciones reales compitiendo sobre una fila, verificando `Conflict` y no `Internal`. Falla — `save`/`is_write_conflict` no implementados (spec R10, R12; el brazo frágil de AD-7).

## Fase 9: GREEN — `save`/`load`/`delete` y Mapeo de Errores — S2 — PR 2

- [x] 9.1 Añadir el accesor `dsn()` — pone en verde 8.1 (EC-6).
- [x] 9.2 Añadir las constantes de sentencia `SELECT_VERSION`/`INSERT_AGGREGATE`/`UPDATE_AGGREGATE` (parametrizadas con `$n`, binding por tupla — matriz de amenazas "Construcción de SQL") e implementar el algoritmo de 7 pasos de `save()` (AD-5): resolver+codificar tenant, transacción real, lectura CAS, fila ausente+esperado distinto de cero ⇒ `Conflict` (EC-1), escritura protegida por versión, relectura si `affected == 0`, commit.
- [x] 9.3 Implementar `is_write_conflict()` (AD-7): `UniqueConstraint`, `TransactionAborted`, `LockAcquisitionFailed`/`DatabaseLocked` ⇒ `Conflict`; el brazo fijado por texto de mensaje `Internal` para la colisión MVCC de reclamo de escritura (EC-7); default de falla ruidosa (`Internal`) para todo lo demás.
- [x] 9.4 Implementar `LOAD_PAYLOAD`/`DELETE_AGGREGATE` y `load()`/`delete()` (AD-6): predicados `=` simples, `NotFound` en fila ausente / cero filas afectadas (spec R7, R8).
- [x] 9.5 Confirmar que 8.1–8.5 pasan todas en verde.

## Fase 10: Verificación — S2 — PR 2

- [x] 10.1 `cargo build -p ego-persistence-stoolap` compila de forma independiente.
- [x] 10.2 `cargo test -p ego-persistence-stoolap` pasa — las 7 pruebas unitarias colocadas en verde.
- [x] 10.3 `cargo run -p xtask -- verify-layers` pasa: el crate nuevo está mapeado, la arista `infrastructure → domain`, sin edición de matriz (R8, proposal).
- [x] 10.4 Puerta de grep: `rg '""' crates/persistence-stoolap/src` devuelve exactamente una línea no-test (AD-3 criterio 1, matriz de amenazas "Fuga del centinela"); ningún token `sqlx`/`PgPool`/`ego-persistence`/`postgres`/de migración en ningún lugar bajo el crate (R7, D-11); ningún token `async`/`tokio`/`block_in_place`/`spawn_blocking` en el crate (D-4).
- [x] 10.5 Confirmar exactamente un `impl Repository<...> for StoolapRepository` y ningún trait propio declarado en el crate (R10, proposal).

## Fase 11: RED — Stoolap se Convierte en el Tercer Sujeto del Harness — S3 — PR 3

- [x] 11.1 Añadir `[dev-dependencies]` a `crates/persistence-stoolap/Cargo.toml`: `ego-testkit = { path = "../testkit" }`, `tempfile = "3"` (AD-9, AD-11 S3).
- [x] 11.2 Crear `crates/persistence-stoolap/tests/repository_conformance.rs`: construir `StoolapRepository<ConformanceAggregate, _>` en una ruta `tempfile::TempDir` nueva, llamar a `ego_testkit::assert_repository_conformance`. Falla al compilar hasta que aterrice 11.1 (AD-11, RED de S3).

## Fase 12: GREEN + Verificación de Todo el Cambio — S3 — PR 3

- [x] 12.1 `cargo test -p ego-persistence-stoolap` pasa — los 11 escenarios del harness en verde contra `StoolapRepository` (spec R1, R2 sujeto 3).
- [x] 12.2 `cargo test --workspace` pasa sin runtime de contenedor disponible; confirmar que no aparece dependencia de Testcontainers/Docker en ningún lugar del workspace raíz (spec R2 combinado con proposal R9, NG-8).
- [x] 12.3 Confirmar que `repository_tenant_scoping_postgres.rs` sigue pasando sin modificar (R11, proposal).
- [x] 12.4 Lectura de diff: `crates/persistence-api/**`, `crates/persistence/**`, `crates/runtime/**`, `crates/effect-store/**` ausentes de la lista de archivos de todo el cambio (R6, R7, proposal NG-4/NG-6/KD-2).
- [x] 12.5 Registrar F-5 y F-6 en la descripción del PR como seguimientos nombrados (R14, proposal); confirmar que KD-1..KD-4 siguen enunciados con precisión y sin tocar.

## Diferido / Fuera de Alcance (deuda nombrada, no tareas)

- **KD-1** — `Snapshot`/`OffsetStore`/`DedupStore` siguen sin un harness de conformidad compartido. Ninguna tarea añade uno (proposal NG-1, F-1).
- **KD-2** — El proveedor Stoolap del effect-store se queda en el default sin fsync. No se cambia aquí (proposal, solo observado).
- **KD-3 → F-5** — `PostgreSQLRepository` ignora `expected_version` en un agregado nuevo (EC-1); `StoolapRepository` reporta conflicto, coincidiendo con la documentación del trait. La reconciliación es su propio cambio con su propia revisión (NG-9/R11).
- **KD-4** — `sync=full` se verifica en el DSN; el fsync genuino se confía a Stoolap, sin prueba de inyección de fallos (design AD-4). Ninguna tarea intenta una prueba de recuperación ante caídas.
- **F-2, F-3, F-4, F-6** — la abstracción de backend, la reubicación de CORE-PERSIST-A2, el renombrado del crate de persistencia, y un modo de sync seleccionable permanecen sin programar (proposal NG-2/NG-4/NG-6, design AD-4 criterio 2).

## Auditoría de Trazabilidad

| Requisito de spec | Tarea(s) que lo cubren |
|---|---|
| R1 — El adaptador existe, hace round-trip | 2.1, 9.2, 9.4, 12.1 |
| R2 — Una suite compartida, tres sujetos | 1.1, 1.2, 2.3, 3.1, 12.1, 12.2 |
| R3 — Aislamiento de tenant incl. systemwide | 2.1, 8.3, 9.2 |
| R4 — Tenant vacío rechazado, nunca coaccionado | 2.1 (escenario 8) |
| R5 — Versión obsoleta ⇒ conflicto veraz | 2.1, 8.4, 9.2 |
| R6 — Nuevo+distinto de cero excluido de la suite compartida, razón documentada | 2.1, 4.4 |
| R7 — Load/delete ausente ⇒ NotFound | 2.1, 9.4 |
| R8 — El delete es permanente | 2.1, 9.4 |
| R9 — Sobrevive un reinicio no limpio | 8.2, 9.2 |
| R10 — Un solo resultado de conflicto para ambas carreras | 8.5, 9.3 |
| R11 — Ningún detalle interno jamás visible | 2.1, 10.4 |
| R12 — Solo garantía de un único proceso | 8.5, 9.3, 12.2 |

**Verificación cruzada de límites de alcance contra proposal NG-1..NG-9 — cero hallazgos.**
Ninguna tarea toca `crates/persistence-api/`, `crates/persistence/`, `crates/runtime/`, o
`crates/effect-store/`; ninguna tarea añade un segundo almacén respaldado por Stoolap, una
abstracción `StorageEngine`/dialecto, o un cuarto backend; ninguna tarea corrige el
comportamiento existente de `PostgreSQLRepository` o `InMemoryRepository`.
