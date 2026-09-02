# Tareas: PROD-014B — Almacenes de Lado de Lectura Durables en PostgreSQL

> Compañero de revisión en español. Fuente de verdad canónica: `tasks.md` (identificadores 1:1).
> TDD estricto (AD-12): toda la suite de conformidad (Fase 3) se escribe en ROJO, contra
> tipos `PostgreSQLOffsetStore`/`PostgreSQLDedupStore` que aún no existen, antes que cualquier
> cuerpo de adaptador (Fase 4). Cada aserción de error nombra la variante específica
> `Fatal`/`Transient`, nunca `is_err()`. La única superficie comprobable por prueba unitaria
> es `is_fatal` (Fase 2) — todo lo demás se prueba contra PostgreSQL real en
> `integration-tests/`.

## Pronóstico de Carga de Revisión

**Las fronteras de PR abajo están confirmadas por el propietario del cambio** (reemplaza la
división inicial Unidad-1/2/3): PR1 = solo esquema, PR2 = adaptadores + `is_fatal` + pruebas
de conformidad, PR3 = adopción en producción. Las etiquetas Fase→PR en todo este documento
coinciden con este mapeo confirmado.

| Campo | Valor |
|-------|-------|
| Líneas cambiadas estimadas | ~620 en total — PR1 ~40 (solo migraciones + registro), PR2 ~500 (`is_fatal` + ambos adaptadores + reexportaciones en `mod.rs` + la suite de conformidad de 8 casos contra PG real), PR3 ~85 (cableado de la app de referencia + docs) |
| Riesgo del presupuesto de 400 líneas | Alto solo para PR2 — una desviación aceptada (ver Condición 5 abajo), no un defecto. PR1 y PR3 quedan cómodamente bajo el presupuesto |
| PRs encadenados recomendados | Sí |
| División sugerida | PR 1 (fundamento de esquema) → PR 2 (adaptadores durables, mapeo de errores, exportaciones, pruebas de conformidad) → PR 3 (adopción en producción: cableado de la app de referencia + docs) |
| Estrategia de entrega | ask-on-risk (valor por defecto de la sesión — no fue provisto por el orquestador en esta corrida) |
| Estrategia de cadena | stacked-to-main — PR2 se rama de PR1, PR3 se rama de PR2 (confirmado por el propietario del cambio; no son tres ramas independientes desde `develop`) |

Decisión necesaria antes de aplicar: No
PRs encadenados recomendados: Sí
Estrategia de cadena: stacked-to-main
Riesgo del presupuesto de 400 líneas: Alto

**Nota de presupuesto de revisión (confirmada, no re-discutida aquí):** las ~500 líneas de
PR2 exceden el presupuesto de 400 principalmente por su propia suite de conformidad contra
PostgreSQL real (~280 líneas). Según la Condición 5 abajo, esto es una desviación aceptada —
la implementación de PR2 nunca se separa de sus propias pruebas para forzarlo bajo el
presupuesto. PR1 y PR3 quedan cada uno bien por debajo de 400 por sí solos.

### Condiciones de la Cadena de PRs (confirmadas por el propietario del cambio)

1. **Cadena apilada**: PR2 se rama de PR1; PR3 se rama de PR2. No son tres ramas
   independientes desde `develop`.
2. **Verde independiente**: cada PR debe compilar y pasar sus propias compuertas en la punta
   de su propia rama — ningún PR depende del código de un PR *posterior* para estar en verde.
3. **Un solo spec, tres unidades de revisión**: los tres PRs son porciones de carga de
   revisión de este único cambio, no capabilities separadas. Ninguna cobertura de requisito o
   escenario se divide ni duplica entre PRs como si fueran cambios independientes — la
   Auditoría de Trazabilidad abajo permanece anclada al cambio como un todo.
4. **Sin estado intermedio engañoso**: ningún PR — incluyendo cualquier commit intermedio
   dentro de un PR — introduce un fallback temporal, un sustituto falso-durable, ni un
   enunciado que implique semántica exactamente-una-vez. `FakeDurableOffsetStore`/
   `FakeDurableDedupStore` (existentes, previos a PROD-014B) nunca se reutilizan como
   sustituto de los adaptadores reales en ningún commit.
5. **El tamaño de PR2 es una desviación aceptada**: PR2 puede exceder moderadamente el
   presupuesto de ~400 líneas debido a su propia suite de conformidad. Esto no es un defecto
   a resolver — la implementación de PR2 nunca se separa de sus propias pruebas solo para
   caber en el presupuesto.

### Unidades de Trabajo Sugeridas

| Unidad | Objetivo | PR | Se rama de | Comando de prueba enfocado | Arnés de runtime | Frontera de rollback |
|------|------|-----|----------------|----------------------|-----------------|-------------------|
| 1 | Fundamento de esquema en PostgreSQL: migraciones `013`/`014`, registradas y ordenadas. Solo pruebas de migración/esquema — explícitamente sin `is_fatal`, sin código de adaptador | PR 1 | `develop` | `cargo test -p ego-persistence migrations` | N/A — solo esquema, ningún comportamiento de adaptador que probar todavía | Borrar `013`/`014` y sus entradas de registro; nada más en el espacio de trabajo las referencia |
| 2 | Adaptadores durables en PostgreSQL: clasificador `is_fatal`, implementaciones `OffsetStore`/`DedupStore`, reexportaciones en `mod.rs`, y la suite completa de conformidad contra PG real (ROJO antes de VERDE, según AD-12) | PR 2 | PR 1 | `cargo test -p ego-persistence postgres::` (unitaria `is_fatal`) + `cargo test -p ego-integration-tests --test read_side_progress_postgres` (conformidad: supervivencia al reinicio, aislamiento, upsert de offset, convergencia de dedup, `is_durable()`) | PostgreSQL real vía `isolated_database()` (excepción arquitectónica documentada, espacio de trabajo `integration-tests/` de nivel raíz) | Borrar `is_fatal`, ambos archivos de adaptador, sus reexportaciones, y el archivo de prueba de conformidad; el esquema de PR 1 permanece válido y sin uso |
| 3 | Adopción en producción: cableado de la app de referencia bajo `Profile::Production`, documentación de la restricción de adopción y operacional, verificación final de trazabilidad | PR 3 | PR 2 | Reejecutar 3.7 (`is_durable()` + aceptación de `Profile::Production`) contra el camino cableado de `main.rs`; `cargo test -p reference-app`; `cargo test --workspace` | `examples/reference-app` componiéndose bajo `Profile::Production` contra un pool de Postgres real | Revertir `ReadSideProgressStores::postgres`, restaurar el `None` de `main.rs` + el comentario retirado "PROD-014A F-1"; PR 1–2 permanecen válidos para cualquier otro host |

## Fase 1: Migraciones (Fundamento) — PR 1

- [x] 1.1 Crear `crates/persistence/src/postgres/migrations/013_create_projection_offsets.sql`: `projection_offsets(projection_id, tag, tenant, offset_value, updated_at)`, `tenant NOT NULL`, `PRIMARY KEY (projection_id, tag, tenant)` (AD-1). Traza: "El Offset Sobrevive a un Reinicio de Proceso", "El Tenant Es Parte Obligatoria de la Identidad del Offset".
- [x] 1.2 Crear `crates/persistence/src/postgres/migrations/014_create_projection_dedup.sql`: `projection_dedup(projection_id, tag, event_id, created_at)`, `PRIMARY KEY (projection_id, tag, event_id)`, sin columna `tenant` (AD-1, AD-7). Traza: "Las Marcas de Dedup Repetidas Convergen a un Solo Registro", "La Identidad de Dedup Es Independiente del Tenant".
- [x] 1.3 Registrar ambas como constantes `include_str!` + dos entradas ascendentes en `migrations.rs::migrations()` (AD-2). No se necesita prueba nueva — ejecutar `cargo test -p ego-persistence migrations` para confirmar que las pruebas existentes `every_migration_file_is_registered_and_every_registration_has_a_file` y `registration_order_ascends_by_numeric_prefix` cubren `013`/`014` (R-4).

## Fase 2: Clasificación de Errores Compartida — `postgres/mod.rs` (Exportaciones y Cableado, Parte A) — PR 2

- [x] 2.1 ROJO: pruebas unitarias `#[cfg(test)]` en `crates/persistence/src/postgres/mod.rs` que afirman que `is_fatal` clasifica valores construidos `sqlx::Error::Database` con SQLSTATE `42P01`/`42703`/`22001`/`23514` y variantes `ColumnDecode`/`Decode` como `true`, y errores de timeout de pool/E-S/protocolo como `false` — sin construir ningún pool (AD-8, AD-12).
- [x] 2.2 VERDE: implementar `pub(crate) fn is_fatal(err: &sqlx::Error) -> bool` con el match exacto de SQLSTATE de AD-8, incluyendo su rustdoc que explica por qué `Transient` es el valor por defecto.

## Fase 3: Pruebas de Integración y Conformidad — ROJO (`integration-tests/tests/infrastructure/read_side_progress_postgres.rs`) — PR 2

Escrita íntegramente antes de que exista cualquier cuerpo de adaptador (AD-12); cada caso
obtiene su base de datos vía `ego_integration_tests::isolated_database()` (D-8, SC-10). Un
fallo de compilación contra `PostgreSQLOffsetStore`/`PostgreSQLDedupStore` aún inexistentes es
el estado ROJO esperado.

- [x] 3.1 ROJO: supervivencia al reinicio — escribir el offset N para `(projection_id, tag, tenant)`, **descartar el almacén y su pool**, abrir un pool *nuevo* contra la misma base, reconstruir el almacén, `read_offset` devuelve N (nunca el estado en proceso). Traza: "El Offset Sobrevive a un Reinicio de Proceso" / escenario "El reinicio retoma desde el último offset persistido"; SC-1, R-3.
- [x] 3.2 ROJO: aislamiento por tenant de offset ausente — existen offsets para el tenant A en `(projection_id, tag)`, el tenant B nunca fue escrito; leer el tenant B en el mismo `(projection_id, tag)` devuelve `None`, nunca el valor del tenant A. Traza: "Las Lecturas de Offset Ausente Están Aisladas por Tenant"; SC-2, G-4.
- [x] 3.3 ROJO: última escritura de offset gana — escribir el offset N, luego escribir el offset M para el mismo `(projection_id, tag, tenant)` sin ninguna coordinación de orden entre ambas escrituras; el valor almacenado pasa a ser M, sin error y sin señal de conflicto para ninguna escritura. Traza: "Las Escrituras de Offset Son 'Última Escritura Gana'"; SC-7 (cada sentencia de offset liga `tenant`, verificado como parte de la preparación de esta prueba).
- [x] 3.4 ROJO: doble marca de dedup secuencial — `mark_seen` llamado dos veces secuencialmente para el mismo `(projection_id, tag, event_id)`; ambas llamadas `Ok`, `SELECT COUNT(*)` es exactamente 1, `seen()` devuelve `true`. Traza: "Las Marcas de Dedup Repetidas Convergen a un Solo Registro".
- [x] 3.5 ROJO: doble marca de dedup concurrente — dos llamadas a `mark_seen` para la misma identidad corren vía `tokio::join!`; ambas `Ok`, `SELECT COUNT(*)` es exactamente 1, `seen()` devuelve `true`. El comentario de documentación de la prueba enuncia explícitamente que esto prueba **convergencia a nivel de almacenamiento de dos llamadas sobre una identidad**, no exclusión de ejecución, no manejo exactamente-una-vez, y no seguridad multi-réplica — la garantía entregada es un-solo-escritor-por-`(projection_id, tag, tenant)` (AD-6; requisito de spec "La Contabilidad Durable de Dedup No Implica Ejecución Exactamente-Una-Vez del Handler"). Traza: "Las Marcas de Dedup Repetidas Convergen a un Solo Registro".
- [x] 3.6 ROJO: independencia de tenant en dedup — `event_id` marcado como visto bajo el tenant A; `seen()` bajo el tenant B para el mismo `(projection_id, tag)` devuelve `true`. Traza: "La Identidad de Dedup Es Independiente del Tenant".
- [ ] 3.7 ROJO: aceptación de durabilidad + perfil de producción — ambos `is_durable()` devuelven `true`; `build_runtime_with(…, Some(ReadSideProgressStores::postgres(pool)))` construye exitosamente bajo `Profile::Production`, sin ningún cambio en la lógica propia de validación de la compuerta. Este caso permanece ROJO hasta que la Fase 6 entregue `ReadSideProgressStores::postgres`; reejecutar en 8.2 para confirmar el VERDE final. Traza: "Ambos Stores de Progreso Se Reportan a Sí Mismos Como Durables", "El Camino de Producción de la Aplicación de Referencia Usa el Par Durable"; SC-5, SC-6.
- [x] 3.8 ROJO: clasificación de migración no aplicada — `read_offset` contra una base sin migración aplicada devuelve `OffsetStoreError::Fatal`, no `Transient` (el reemplazo de AD-8 para un método `probe()`, AD-9).

## Fase 4: Adaptadores — VERDE (`crates/persistence/src/postgres/`) — PR 2

- [x] 4.1 VERDE: crear `read_side_offset.rs` — `PostgreSQLOffsetStore { pool: PgPool }`, `pub fn new(pool: PgPool)`, `Debug` manual (solo pool), `is_durable() -> true`, `write_offset` como el upsert de AD-3 (`ON CONFLICT (projection_id, tag, tenant) DO UPDATE SET offset_value = EXCLUDED.offset_value, updated_at = NOW()`), `read_offset` como la búsqueda escalar `fetch_optional` de AD-4; ambos mapean errores vía `is_fatal` a `OffsetStoreError::{Fatal,Transient}`. Pone en VERDE 3.1, 3.2, 3.3, 3.8 (3.7 parcialmente — solo la mitad de offset).
- [x] 4.2 VERDE: crear `read_side_dedup.rs` — `PostgreSQLDedupStore { pool: PgPool }`, `pub fn new(pool: PgPool)`, `Debug` manual, `is_durable() -> true`, `mark_seen` como el `INSERT … ON CONFLICT (projection_id, tag, event_id) DO NOTHING` de AD-5, `seen` como la búsqueda puntual por clave primaria de AD-5; ambos mapean errores vía `is_fatal` a `DedupStoreError::{Fatal,Transient}`. Pone en VERDE 3.4, 3.5, 3.6 (3.7 parcialmente — solo la mitad de dedup).

## Fase 5: Exportación de Adaptadores — `postgres/mod.rs` (Exportaciones y Cableado, Parte B) — PR 2

- [x] 5.1 Agregar `pub use read_side_offset::PostgreSQLOffsetStore;` y `pub use read_side_dedup::PostgreSQLDedupStore;` a `crates/persistence/src/postgres/mod.rs` (IS-4). Confirma que 3.1–3.6 y 3.8 compilan y pasan contra los tipos de adaptador reales.

## Fase 6: Cableado de Producción en la App de Referencia (`examples/reference-app/`) — PR 3

- [ ] 6.1 Agregar `pub fn postgres(pool: PgPool) -> Self` a `ReadSideProgressStores` en `src/read_side/mod.rs`, junto a `in_memory()`/`fake_durable()`, cableando `Arc::new(PostgreSQLOffsetStore::new(pool.clone()))` / `Arc::new(PostgreSQLDedupStore::new(pool))` (AD-10). El rustdoc enuncia la restricción de adopción de un-solo-escritor-por-`(projection_id, tag, tenant)` textualmente según el fragmento de AD-10. Traza: "La Restricción de Adopción de Escritor Único Está Documentada a Nivel de Adaptador"; IS-5, IS-8.
- [ ] 6.2 En `src/main.rs`: tomar `pool.clone()` **antes** de `EntityEventStores::open(pool)` (resuelve EC-2), construir `read_side_progress = ReadSideProgressStores::postgres(pool.clone())` inmediatamente después de `migrations::run(&pool)` (línea 77), y pasar `Some(read_side_progress)` a `build_runtime_with(...)`, eliminando el `None` + el comentario retirado "PROD-014A F-1". Traza: "El Camino de Producción de la Aplicación de Referencia Usa el Par Durable"; IS-6, SC-6.

## Fase 7: Documentación de la Restricción de Adopción y Operacional — PR 3

- [ ] 7.1 Rustdoc en `PostgreSQLOffsetStore`/`PostgreSQLDedupStore` (o un doc de módulo compartido en `read_side_offset.rs`/`read_side_dedup.rs`): enunciar la restricción de adopción de un-solo-escritor-por-`(projection_id, tag, tenant)` y que ninguna configuración de proyección multi-réplica está oficialmente soportada (D-7, IS-8, SC-8, SC-12). Traza: "La Restricción de Adopción de Escritor Único Está Documentada a Nivel de Adaptador", "La Prevención de Ejecución Doble del Handler Descansa en una Restricción de Adopción de Escritor Único, Explícita y No Forzada".
- [ ] 7.2 Nota operacional en el rustdoc de `PostgreSQLDedupStore`: `projection_dedup` crece de forma ilimitada y monótona con los eventos únicos procesados; ningún purgado/TTL/desalojo se entrega en este cambio; el conteo de filas es una señal a observar, no una sorpresa (D-4, L-4, AD-11). Traza: "El Crecimiento del Almacenamiento de Dedup Es Ilimitado en Esta Capability".
- [ ] 7.3 Crear `crates/persistence/README.md` (no existe hoy) documentando el par durable de lado de lectura, y actualizar la sección `Profile::Production` / Persistence Completeness Rule de `ARCHITECTURE.md` (~línea 197-210) con la misma restricción de adopción de un solo escritor más el seguimiento nombrado **PROD-014C — Reclamación Atómica de Eventos del Read-Side** (D-7, F-1, SC-9). Traza: "La Contabilidad Durable de Dedup No Implica Ejecución Exactamente-Una-Vez del Handler", "La Prevención de Ejecución Doble del Handler Descansa en una Restricción de Adopción de Escritor Único, Explícita y No Forzada", "La Brecha de Concurrencia Tiene un Seguimiento Nombrado y Distinto".

## Fase 8: Verificación Final — PR 3

- [ ] 8.1 `cargo test --workspace` cero fallas nuevas; `cargo clippy --workspace -- -D warnings` limpio; confirmar que ninguna función tocada excede complejidad cognitiva 10.
- [ ] 8.2 Reejecutar la suite completa de conformidad (`cargo test -p ego-integration-tests --test read_side_progress_postgres`); confirmar que 3.1–3.8 están todas en VERDE, incluyendo 3.7 ahora que la Fase 6 existe.
- [ ] 8.3 Confirmación por lectura del diff (sin cambio de código): SC-7 — cada `$N` ligado, ninguna interpolación en ningún lado; SC-11 — `crates/domain/src/read_side/`, la compuerta/registro de `crates/service-sdk`, y `crates/runtime/src/read_side/scheduler.rs` no aparecen en ninguna lista de archivos de este cambio.

## Auditoría de Trazabilidad

Los 13 requisitos de spec mapeados a al menos una tarea que los cubre:

| Requisito | Capability | Tarea(s) que lo cubren |
|---|---|---|
| El Offset Sobrevive a un Reinicio de Proceso | `read-side-durable-progress` | 1.1, 3.1, 4.1 |
| Las Lecturas de Offset Ausente Están Aisladas por Tenant | `read-side-durable-progress` | 1.1, 3.2, 4.1 |
| Las Marcas de Dedup Repetidas Convergen a un Solo Registro | `read-side-durable-progress` | 1.2, 3.4, 3.5, 4.2 |
| La Identidad de Dedup Es Independiente del Tenant | `read-side-durable-progress` | 1.2, 3.6, 4.2 |
| Las Escrituras de Offset Son "Última Escritura Gana" | `read-side-durable-progress` | 3.3, 4.1 |
| Ambos Stores de Progreso Se Reportan a Sí Mismos Como Durables | `read-side-durable-progress` | 3.7, 4.1, 4.2 |
| El Tenant Es Parte Obligatoria de la Identidad del Offset | `read-side-durable-progress` | 1.1, 4.1 |
| El Crecimiento del Almacenamiento de Dedup Es Ilimitado en Esta Capability | `read-side-durable-progress` | 7.2 (ningún código de limpieza se entrega en ninguna de las Fases 1–6, confirmado en 8.3) |
| El Camino de Producción de la Aplicación de Referencia Usa el Par Durable | `read-side-durable-progress` | 3.7, 6.1, 6.2 |
| La Restricción de Adopción de Escritor Único Está Documentada a Nivel de Adaptador | `read-side-durable-progress` | 6.1, 7.1 |
| La Contabilidad Durable de Dedup No Implica Ejecución Exactamente-Una-Vez del Handler | `read-side` (AGREGADA) | 3.5, 7.3 |
| La Prevención de Ejecución Doble del Handler Descansa en una Restricción de Adopción de Escritor Único, Explícita y No Forzada | `read-side` (AGREGADA) | 6.1, 7.1, 7.3 |
| La Brecha de Concurrencia Tiene un Seguimiento Nombrado y Distinto | `read-side` (AGREGADA) | 7.3 |

**Verificación cruzada de frontera de alcance contra los No-Objetivos de la spec y las
referencias OOS del diseño — cero hallazgos.** Ninguna tarea de esta lista toca: retención,
TTL o limpieza de dedup (explícitamente descartado — la documentación de la Fase 7 solo
enuncia la limitación, ningún camino de código existe en ninguna fase); reclamación atómica de
eventos/reserva, elección de líder, locks, leases o fencing (OOS-2 — ninguna tarea agrega uno;
la prueba de mark_seen concurrente de la Fase 3 (3.5) descarta explícitamente probar
exclusión); detección de multi-réplica de ningún tipo (OOS-2/OOS-7 — ninguna tarea agrega
una); ni ningún backend distinto de PostgreSQL (OOS-4 — cada tarea apunta a
`crates/persistence/src/postgres/`). Todo esto queda reservado para **PROD-014C —
Reclamación Atómica de Eventos del Read-Side** (nombrado en 7.3, no implementado aquí) o para
el backlog (retención F-2), según D-4, D-7, OOS-2, OOS-3, OOS-4.
