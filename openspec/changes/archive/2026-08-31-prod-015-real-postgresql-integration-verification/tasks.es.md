# Tareas: PROD-015 — Verificación de integración real con PostgreSQL

> Compañero de revisión en español. Fuente canónica: `tasks.md` (mismos IDs de tarea, mismo
> orden, misma evidencia). Se apoya en `proposal.md`, `spec.md` y `design.md`, todos finales y
> revisados de forma cruzada; ninguna decisión registrada allí se vuelve a discutir aquí.

## Cómo leer este archivo

Cada tarea indica: qué archivos toca, de qué depende, si puede ejecutarse en paralelo con otras
tareas, qué requisito de spec (`IS-#` / `SC-#`) o decisión de diseño (`AD-#`) satisface, pasos
concretos bajo TDD (ROJO → VERDE, según `skills/testing-tdd/SKILL.md`) y **evidencia
verificable** — un comando a ejecutar, una aserción que debe cumplirse, o un valor a capturar.
"El test pasa" por sí solo nunca es evidencia suficiente.

Cada archivo de test nuevo también lleva, en su propio comentario de documentación, el
invariante que demuestra y por qué en proceso no puede mostrarlo (`IS-7`, regla de admisión 4)
— indicado una vez por archivo abajo y se espera que llegue textualmente como comentario de
documentación en Rust, no parafraseado de otra forma en el código.

Todos los archivos nuevos/modificados permanecen dentro de `integration-tests/` o los tres
archivos que este cambio toca directamente
(`crates/persistence/src/postgres/reservation.rs`,
`crates/persistence/src/postgres/aggregate_type_backfill.rs`,
`crates/persistence/src/postgres/event_store.rs`) — y sólo como mutaciones **temporales,
revertidas por diseño** para IS-8 (Grupo 2b/4b/1, tareas de mutación). Ningún archivo de
producción queda modificado por este cambio (D-8, Plan de reversión).

## Leyenda de prioridad

- **P0** — la carrera de fencing con múltiples contendientes (IS-3) y la carrera de append
  N-way (IS-2, sólo el alcance de relectura post-`23505`), más la prueba de mutación de IS-3.
  Requerido por la puerta 5 de esta fase.
- **P1** — todo lo demás en alcance: IS-1 (fundacional, el más barato, cierra AC10, retira
  IS-6), IS-4, IS-5, IS-9, y las pruebas de mutación IS-8 para IS-4/IS-6 (M2, M3).
- IS-1 aparece en el Grupo 1 antes que los grupos P0 porque es el primer bloque sin
  dependencias y el más barato (`design.md` "Approach": "IS-1 se deja deliberadamente primero
  y es el más barato") y un PR completo por sí solo; la etiqueta de prioridad y el orden de
  entrega son independientes.

---

## Grupo 0 — Infraestructura compartida (prerrequisito secuencial para IS-2, IS-3, IS-9)

### T-00.1 — Promover `wait_until_blocked` a `src/lib.rs`

- **Archivos:** `integration-tests/src/lib.rs` (función nueva), `integration-tests/tests/infrastructure/fencing_window_postgres.rs` (sólo el punto de llamada)
- **Depende de:** nada
- **Bloquea:** T-02.* (IS-3), T-03.* (IS-2/IS-5)
- **Paralelo con:** T-00.2, T-01.*
- **Satisface:** `design.md` AD-3 (prerrequisito para IS-2/IS-3; no es un `IS-#` de spec por sí mismo)
- **Pasos:**
  1. ROJO: añadir `pub async fn wait_until_blocked(observer: &PgPool, statement_like: &str, expected: usize)` a `src/lib.rs`, extraída del helper privado `wait_until_contender_is_blocked` de `fencing_window_postgres.rs`, con las dos correcciones que AD-3 exige: (a) añadir `AND datname = current_database()` al predicado de `pg_stat_activity` — la visibilidad a nivel de clúster entre hasta 8 bases de datos aisladas convierte esta omisión en un riesgo de falso positivo en cuanto IS-3 añada un segundo test bloqueante; (b) reducir la coincidencia de la sentencia de un fragmento de nombre de tabla desnudo a un fragmento de sentencia (p. ej. `'%UPDATE operation_reservations%'`), de modo que un backend contado ya haya pasado, demostrablemente, sus sentencias previas al bloqueo. Sondea con un plazo explícito y falla el test al llegar al plazo — el `sleep` interno es un intervalo de sondeo, nunca un timeout que sustituya a una condición. `fencing_window_postgres.rs` aún no la llama (ROJO: compila, pero su propio helper privado queda ahora como código muerto/duplicado — un estado deliberadamente temporal).
  2. VERDE: reemplazar el punto de llamada al sondeo privado de `fencing_window_postgres.rs` por `ego_integration_tests::wait_until_blocked(...)`; eliminar la copia privada ahora muerta. El comentario de documentación y todas las aserciones de ese archivo permanecen sin cambios (tabla de cambios de archivos de design.md).
- **Evidencia:** `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` — `fencing_window_postgres` sigue pasando con aserciones idénticas byte a byte (`owner_id`, `StaleOwner`, verificaciones de fencing-token); `git diff integration-tests/tests/infrastructure/fencing_window_postgres.rs` muestra que sólo cambió la línea del punto de llamada al sondeo, el comentario de documentación queda intacto.
- **Estado:** [x] Hecho — `wait_until_blocked` incorporada a `src/lib.rs`; `fencing_window_postgres.rs` la llama, copia privada eliminada. Ejecución real con contenedores: 43 pasados, 1 ignorado preexistente, 0 fallidos; las aserciones de `fencing_window_postgres` sin cambios.

### T-00.2 — Extensión del runner con un segundo contenedor PG14

- **Archivos:** `integration-tests/src/main.rs` (modificado), `integration-tests/src/lib.rs` (modificado — añade `pg14_database()`)
- **Depende de:** nada
- **Bloquea:** T-05.* (IS-9)
- **Paralelo con:** T-00.1, T-01.*
- **Satisface:** `design.md` AD-6 (prerrequisito para IS-9)
- **Pasos:**
  1. ROJO: añadir `pub async fn pg14_database() -> IsolatedDatabase` a `src/lib.rs` — crea una base de datos en el contenedor PG14 y ejecuta `ego_persistence::postgres::migrations::run()` contra ella **directamente, in situ, sin plantilla, sin clon** (la propia ejecución de la migración es el invariante que demuestra IS-9, según AD-6). Compila; nada la llama todavía.
  2. VERDE: extender `main.rs` para arrancar un segundo contenedor `Postgres::default().with_tag("14")`, publicar `EGO_IT_PG14_HOST` / `EGO_IT_PG14_PORT` al proceso hijo de tests, y liberar **ambos** contenedores (`.rm().await`) en cada camino de salida — éxito, fallo de test y la ruta preexistente de limpieza segura ante panic/unwind — dentro del runtime de Tokio aún activo, antes de devolver el código de salida de la suite. Extender la línea de reporte de tiempos existente con los instantes de aprovisionamiento y liberación de PG14.
- **Evidencia:** ejecutar la suite una vez limpia y confirmar mediante el propio listado del runtime de contenedores (p. ej. `docker ps -a` / el contexto activo de Docker/`colima`) que no quede ningún contenedor tras la salida; la línea de tiempos impresa por el runner nombra dos instantes de aprovisionamiento (PG16, PG14) y ambos instantes de liberación.
- **Estado:** [x] Hecho — `pg14_database()` añadida a `src/lib.rs`; `main.rs` arranca un segundo contenedor `Postgres::default().with_tag("14")` y libera ambos de forma independiente. Ejecución real confirmó vía `docker ps -a` que no queda ningún contenedor tras la salida; línea de tiempos: `provisioned in 8.11s (PG16) / 31.67s (PG14) · template migrated at 31.80s · suite finished at 36.14s · reclaimed at 36.38s`. Brecha menor de redacción de evidencia: la línea de tiempos reporta un único instante combinado `reclaimed_at` en lugar de dos instantes de liberación separados, aunque ambas llamadas `.rm().await` son independientes y se verifican de forma independiente.

---

## Grupo 1 — IS-1 (fundacional, cierra AC10) + retiro de IS-6 (P1, se entrega primero)

### T-01.1 — ROJO: `durable_store_conformance_postgres.rs`, mitad de event-store

- **Archivo:** nuevo `integration-tests/tests/infrastructure/durable_store_conformance_postgres.rs`
- **Depende de:** nada (independiente del Grupo 0)
- **Paralelo con:** T-00.*, T-02.*, T-03.*, T-04.*
- **Satisface:** IS-1 (rama de event store), IS-6 (retirado hacia esta misma ejecución según el veredicto confirmado de D-4/AD-4 — sin test ni fila de ledger separados)
- **Pasos:**
  1. ROJO: `db = isolated_database(); pool = connect(db.url(), 4)` (`max_connections >= 2` es determinante según AD-4 — con un pool de 1, `load()` se quedaría esperando la conexión retenida por la unidad de trabajo abierta y fallaría como timeout de pool, no como fallo de aislamiento; se fija en 4). `let mut store = PostgreSQLEventStore::open(pool, deserialize).await?;` luego `assert_event_store_conformance(&mut store, |kind| ConformanceEvent { .. }).await;` con un fixture local `ConformanceEvent` (no un contrato re-derivado — la copia de `crates/infrastructure` es privada de otro objetivo de test de otro crate, según AD-4). El archivo aún no está registrado como módulo: `cargo test --manifest-path integration-tests/Cargo.toml --test ledger` falla, correctamente, nombrando el archivo no registrado.
  2. Comentario de documentación (textual, según regla de admisión 4): declara el invariante — `PostgreSQLEventStore` satisface las mismas definiciones de `assert_event_store_conformance` que satisfacen los adaptadores en memoria, incluyendo que un append preparado pero no confirmado en la conexión retenida por `PostgreSQLEventStore` es invisible para un `store.load()` emitido desde una conexión distinta del pool, y que una unidad de trabajo descartada sin `commit` no persiste nada (IS-6, demostrado aquí según D-4, sin test separado) — y por qué en proceso no puede mostrarlo: ningún doble en memoria tiene una transacción real, una segunda conexión real del pool, ni visibilidad real entre conexiones bajo `READ COMMITTED`.
- **Evidencia:** diferida a T-01.3 (módulo aún no registrado — el ledger está deliberadamente en rojo aquí).
- **Estado:** [x] Hecho — archivo creado con el test de event-store y el comentario de documentación textual (adaptado a ambas mitades; T-01.2 se incorporó al mismo archivo). ROJO confirmado: `cargo test --manifest-path integration-tests/Cargo.toml --test ledger` falló, nombrando `durable_store_conformance_postgres` como no registrado y no documentado, antes de T-01.3.

### T-01.2 — VERDE: `durable_store_conformance_postgres.rs`, mitad de reservation-store

- **Archivo:** mismo archivo que T-01.1, segundo `#[tokio::test]`
- **Depende de:** T-01.1 (mismo archivo, un solo escritor)
- **Satisface:** IS-1 (rama de reservation store)
- **Pasos:**
  1. `let pool = &connect(db.url(), 4).await;` — la fábrica que exige el harness debe ser `Copy`, lo que prohíbe capturar un `PgPool` propio; una referencia compartida sí es `Copy` (AD-4 lo verificó contra la firma del harness, que es **async**, no la forma síncrona que indicaba D-5). `assert_reservation_store_conformance(|| async move { sqlx::query("TRUNCATE operation_reservations").execute(pool).await.unwrap(); let clock = Arc::new(TestClock::new(epoch())); (PostgresOperationReservationStore::new(pool.clone(), clock.clone()), clock) }).await;` `TRUNCATE` es el reset que necesitan las 21 llamadas a `fresh()` del harness — el store posee exactamente una tabla, así que truncarla es suficiente y barato (AD-4; una base de datos aislada nueva por llamada se rechazó allí por implicar 21 `CREATE DATABASE` serializados sin ganancia de aislamiento).
  2. Adición al comentario de documentación para esta segunda función: invariante = conformidad durable de fencing/lease bajo `UPDATE`s condicionales reales; por qué no en proceso = las aserciones de fencing/CAS del harness necesitan una fila real y una comparación condicional real, que un store simulado no puede falsear del modo que esta suite está construida para detectar.
- **Evidencia:** diferida a T-01.3.
- **Estado:** [x] Hecho — segundo `#[tokio::test]` añadido al mismo archivo, cierre `pool: &PgPool` (Copy, según AD-4), `TRUNCATE operation_reservations` como reset.

### T-01.3 — VERDE: registrar módulo + fila del ledger del README para IS-1/IS-6

- **Archivos:** `integration-tests/tests/infrastructure.rs` (añadir `mod durable_store_conformance_postgres;`), `integration-tests/README.md` (nueva categoría "Durable-adapter conformance", IS-1 citando ambas funciones `#[tokio::test]`; una frase junto a ella indicando que IS-6 queda demostrado por esta misma ejecución según D-4, sin fila separada)
- **Depende de:** T-01.1, T-01.2
- **Pasos:** registrar el módulo; añadir la fila del ledger exactamente como una fila de tabla con la ruta como código en línea (el guard sólo cuenta filas, según la regla de análisis documentada por el propio `integration-tests/README.md`); actualizar el contador `Total infrastructure tests`.
- **Evidencia:** `cargo test --manifest-path integration-tests/Cargo.toml --test ledger` pasa sin desviación; `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` muestra que `postgres_event_store_conformance` y `postgres_reservation_store_conformance` pasan, con un número de aserciones no menor que el de los llamadores en memoria existentes (`crates/infrastructure/tests/in_memory_event_store_conformance.rs`, `crates/testkit/src/reservation.rs`) — las mismas definiciones, no un conjunto debilitado.
- **Nota D-8:** cualquier fallo de conformidad que aparezca aquí probablemente se localice en la rama de fingerprint en conflicto de `PostgresEventStoreUnitOfWork::confirm_receipt` ("nota D-8" del propio AD-4 — nunca se había ejecutado contra PostgreSQL real). **Ningún arreglo está preautorizado.** Un defecto descubierto aquí se convierte en una spec de seguimiento con nombre propio (la excepción de arreglo pequeño de D-8 cubre explícitamente sólo IS-2 e IS-4, nunca IS-1/IS-6).
- **Estado:** [x] Hecho — módulo registrado en `tests/infrastructure.rs`, nueva sección "Durable-adapter conformance" + fila añadida a `README.md` con la nota IS-6, contador `Total infrastructure tests` actualizado 16 → 17. `cargo test --test ledger`: 9/9 pasados. Ejecución real con contenedores: 45 pasados (43 preexistentes + 2 nuevos), 1 ignorado preexistente, 0 fallidos — tanto `postgres_event_store_conformance` como `postgres_reservation_store_conformance` pasaron en la primera ejecución real; no apareció ningún fallo de conformidad, así que la ubicación presumible de la nota D-8 nunca se ejerció y no hace falta spec de seguimiento para T-01.3.

### T-01.4 — Prueba de mutación IS-8 para IS-1/IS-6 (M3), P1

- **Archivos (mutados temporalmente y luego revertidos):** `crates/persistence/src/postgres/event_store.rs`; **Adición al ledger:** tabla de mutaciones ya existente en `integration-tests/README.md`
- **Depende de:** T-01.3 (el test debe existir y estar en verde antes de poder demostrarse que falla)
- **Satisface:** IS-8 (escenario "neutralizar la atomicidad transaccional/de unidad-de-trabajo hace fallar el test correspondiente", spec.md), SC-8
- **Pasos (receta exacta de 7 pasos de AD-7):**
  1. `shasum -a 256 crates/persistence/src/postgres/event_store.rs` — registrar ANTES.
  2. En `PostgresEventStoreUnitOfWork::append`, cambiar `.execute(&mut *self.tx)` → `.execute(&self.pool)` (desvía la escritura fuera de la transacción retenida).
  3. `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
  4. Registrar: esperado — `durable_store_conformance_postgres` falla en "un append no confirmado no debe ser visible para un lector"; los llamadores de conformidad en memoria (código de producción no afectado) permanecen en verde. Registrar el mensaje exacto de la aserción que falla.
  5. `git checkout -- crates/persistence/src/postgres/event_store.rs`.
  6. `shasum -a 256` — DEBE ser igual al valor del paso 1.
  7. Reejecutar el paso 3 como control negativo: todo en verde.
- **Evidencia:** una fila nueva (`M3`) en la tabla de mutaciones de `integration-tests/README.md` con el par SHA-256 ANTES/DESPUÉS-de-restaurar (iguales) y el nombre exacto del test que falla + el mensaje de aserción del paso 4.
- **Estado:** [x] Hecho — nueva tabla "IS-8 mutation proofs" añadida a `README.md` (no existía ninguna; la "tabla de mutaciones" preexistente prueba el propio `tests/ledger.rs`, una obligación distinta, así que se añadió una sección nueva en vez de reutilizarla). SHA-256 antes = `9dc88a462…` = después de restaurar (iguales, confirmado). Ejecución mutada: `durable_store_conformance_postgres::postgres_event_store_conformance` falló exactamente como se predijo ("an uncommitted append must not be visible to a reader, saw 1 event(s)"), todos los demás tests, incluyendo `postgres_reservation_store_conformance`, permanecieron en verde. Reejecución de control negativo: 45 pasados, 1 ignorado preexistente, 0 fallidos.

---

## Grupo 2 — IS-3, P0: carrera de fencing con múltiples contendientes + su prueba de mutación

### T-02.1 — ROJO: `lease_contention_postgres.rs`, carrera de seis contendientes

- **Archivo:** nuevo `integration-tests/tests/infrastructure/lease_contention_postgres.rs`
- **Depende de:** T-00.1 (`wait_until_blocked`)
- **Paralelo con:** T-01.*, T-03.*, T-04.*
- **Satisface:** IS-3, SC-3
- **Pasos:**
  1. ROJO: orquestar exactamente según la forma de AD-3 — una transacción `holder` ejecuta `SELECT owner_id … WHERE operation_key = $1 FOR UPDATE` sobre la fila con lease expirado y retiene el bloqueo de fila; lanzar seis tareas contendientes, cada una llamando a `store.reserve(owner_b_i, …)` contra un pool del store con `max_connections >= 6` (el diseño lo fija en 8); el `INSERT … ON CONFLICT DO NOTHING` de cada contendiente (0 filas, no espera) y su `SELECT` sencillo (MVCC, ve el lease expirado) se completan, luego su `UPDATE … WHERE fencing_token = T AND lease_until <= now` se bloquea en el bloqueo de fila del holder; el test llama a `wait_until_blocked(observer, "%UPDATE operation_reservations%", 6)` con un plazo explícito antes de liberar al holder — sin este sondeo el test sería probabilístico, ya que seis contendientes podrían resolverse en serie con las aserciones aun así pasando sin probar nada (razón que el propio AD-3 declara). `holder.commit()`. Esperar los seis resultados de contendientes.
  2. Comentario de documentación (textual): invariante — seis contendientes reales compitiendo por un lease expirado dejan exactamente un ganador `TakenOver` y el fencing token avanza en exactamente uno, nunca en el número de contendientes; por qué no en proceso — forzar a seis sentencias `UPDATE` reales a bloquearse genuinamente en un bloqueo de fila real, con un sondeo determinista que demuestre que los seis leyeron el lease expirado antes de que ninguno escribiera, no es expresable sin un bloqueo de fila real de PostgreSQL.
  3. VERDE: registrar el módulo; añadir la fila del README bajo "PostgreSQL concurrency invariants".
- **Evidencia:** exactamente un `TakenOver`; exactamente cinco `OtherInProgress`; `fencing_token == T + 1` (nunca `T + 6`); el `owner_id` ganador coincide con el único resultado `TakenOver`. `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` — bloque ≤1–2 min (SC-9).
- **Estado:** [x] Hecho — el guardián del ledger confirmó primero ROJO (`every_test_on_disk_is_registered_as_a_module` y `..._is_accounted_for_in_the_ledger` fallaron, nombrando `lease_contention_postgres`); módulo registrado en `tests/infrastructure.rs`, fila añadida a la tabla "PostgreSQL concurrency invariants" del README, bloque Budget actualizado (2 → 3 invariantes de concurrencia, 17 → 18 total). Ejecución con contenedor real: `lease_contention_postgres::six_contenders_racing_one_expired_lease_leave_exactly_one_winner` pasó — exactamente un `TakenOver`, exactamente cinco `OtherInProgress`, `fencing_token == a_token + 1`, el owner/token ganador coincide con la fila leída directamente. Suite completa: 46 pasaron (43 preexistentes + 2 del Grupo 1 + 1 nuevo de esta tarea), 1 preexistente ignorado, 0 fallos; la suite terminó en 13.62s — muy dentro del presupuesto ≤1–2 min por bloque (SC-9).

### T-02.2 — Prueba de mutación IS-8 para IS-3 (M1a / M1b / M1c), P0

- **Archivos (mutados temporalmente y luego revertidos):** `crates/persistence/src/postgres/reservation.rs`; **Adición al ledger:** tabla de mutaciones de `integration-tests/README.md` (tres filas)
- **Depende de:** T-02.1 (el test debe existir y estar en verde), T-00.1 (`fencing_window_postgres` ya usando el helper compartido)
- **Satisface:** IS-8 (escenario "neutralizar el mecanismo de fencing hace fallar el test de fencing", spec.md), SC-8 — **según la corrección de AD-7**, radio de impacto, no verdor global, para M1c
- **Pasos — tres aplicaciones separadas de la receta de 7 pasos de AD-7, una por fila, nunca combinadas en una sola edición:**
  1. **M1a** — neutralizar el predicado de expiración del lease. Registrar hash ANTES. Editar el `.bind(now)` del `UPDATE` de toma de control → `.bind(now - chrono::Duration::days(3650))`, que neutraliza `lease_until <= $7`. Ejecutar la suite. Registrar: esperado — `fencing_window_postgres` **falla**; `lease_contention_postgres` permanece en verde (el predicado CAS por sí solo sigue siendo suficiente). Restaurar; verificar igualdad de hash; reejecución de control negativo, todo en verde.
  2. **M1b** — neutralizar el CAS del fencing-token. Registrar hash ANTES. Editar `AND fencing_token = $6` → `AND $6 = $6`. Ejecutar la suite. Registrar: esperado — **toda la suite permanece en verde** (el predicado de expiración del lease por sí solo sigue siendo suficiente; el propio comentario de `reservation.rs:300-309` ya lo afirma, y esta fila lo vuelve a medir en vez de heredar esa afirmación). Restaurar; verificar; reejecución de control negativo.
  3. **M1c** — neutralizar ambos predicados a la vez. Registrar hash ANTES. Aplicar simultáneamente las ediciones de M1a y M1b. Ejecutar la suite. Registrar: esperado — `lease_contention_postgres` **falla**, observando seis resultados `TakenOver` donde se exige exactamente uno; `fencing_window_postgres` **también falla**, porque ambos tests cargan el mismo predicado. Éste es el caso en el que la cláusula literal de SC-8 "y la suite preexistente permanece en verde" no se cumple; la afirmación demostrable y más estrecha — registrada aquí textualmente — es: *el test nuevo falla, y ningún test que no ejercite este predicado falla*. Restaurar; verificar igualdad de hash; reejecución de control negativo, todo en verde.
- **Evidencia:** tres filas del ledger del README (`M1a`, `M1b`, `M1c`), cada una con su par SHA-256 ANTES/DESPUÉS-de-restaurar (iguales) y el nombre exacto del test/tests que fallan + mensaje(s) de aserción de su ejecución. La fila de M1c indica explícitamente el marco de radio de impacto anterior, no "la suite permanece en verde".
- **Estado:** [x] Hecho, con una edición corregida detallada abajo — tres filas añadidas a la nueva tabla de pruebas de mutación IS-8 del README (la tabla de T-01.4), cada una siguiendo la receta completa de 7 pasos de AD-7, aplicadas por separado. Hash ANTES para las tres: `e69b9bf6cefb6a51fd60ad1af9e75c1fd5fe05a1466f7d4be41425a8221118f3` (== después de restaurar, igualdad confirmada cada vez). **M1a — signo corregido respecto al texto literal de esta tarea**: `.bind(now)` → `.bind(now - chrono::Duration::days(3650))` (una fecha pasada) se probó primero y empíricamente rompe el predicado en el sentido *opuesto* — hace que `lease_until <= $7` sea casi siempre **falso**, rechazando toda toma de control legítima, y falló 5 tests en toda la suite (`durable_store_conformance_postgres::postgres_reservation_store_conformance`, `dual_aggregate_crash_recovery_postgres`, `takeover_fencing_postgres`, `fencing_window_postgres`, `lease_contention_postgres`), contradiciendo el radio de impacto que esta misma tarea predice. Revertido, hash reconfirmado igual, y reaplicado como `.bind(now + chrono::Duration::days(3650))` (una fecha futura), que hace el predicado casi siempre **verdadero** — la dirección correcta de "neutralizar". Esa versión coincidió exactamente con la predicción: solo falló `fencing_window_postgres` ("Got TakenOver(…)" donde se exigía un rechazo), 45 pasaron/1 falló/1 ignorado, `lease_contention_postgres` permaneció en verde. Restaurado; hash igual; reejecución de control negativo todo en verde. **M1b**: `AND fencing_token = $6` → `AND $6 = $6`; toda la suite permaneció en verde como se predijo, 46 pasaron/0 fallaron/1 ignorado, re-midiendo la propia afirmación del comentario de `reservation.rs:300-309`. Restaurado; hash igual; reejecución de control negativo todo en verde. **M1c**: M1a (corregido, `+`) y M1b aplicados juntos; tanto `fencing_window_postgres` como `lease_contention_postgres` fallaron como se predijo — este último observando que los seis contendientes reportan `TakenOver` (`left: 6, right: 1`) — 44 pasaron/2 fallaron/1 ignorado, coincidiendo textualmente con el marco de radio de impacto (no verdor global, según SC-8/AD-7). Restaurado; hash igual; reejecución de control negativo todo en verde (46/0/1). `git diff --stat` sobre `reservation.rs` está vacío — ningún archivo de producción queda modificado.

---

## Grupo 3 — IS-2 (P0, sólo alcance post-`23505`) + IS-5 (P1, mismo archivo)

### T-03.1 — ROJO: `events_identity_race_postgres.rs`, carrera de append N-way de IS-2

- **Archivo:** nuevo `integration-tests/tests/infrastructure/events_identity_race_postgres.rs`
- **Depende de:** T-00.1 (`wait_until_blocked`)
- **Paralelo con:** T-01.*, T-02.*, T-04.*
- **Satisface:** IS-2 — **acotado exactamente a la rama de relectura post-`23505`** (spec.md, requisito MODIFICADO "Effective Uniqueness on the Event Stream Identity", tercer escenario), nunca la comprobación previa de versión-esperada-obsoleta de un solo llamador, que la ejecución de conformidad de IS-1 ya ejercita (`crates/testkit/src/event_store.rs:124-149`) — reafirmarla aquí violaría la regla de admisión 3 (sin duplicación). Segunda cláusula de SC-2.
- **Pasos:**
  1. ROJO: `holder: BEGIN; LOCK TABLE events IN EXCLUSIVE MODE` (bloquea `INSERT`, permite `SELECT` simple); lanzar cuatro tareas de carrera, cada una llamando a `store.append(type, id, tenant, 0, [event])` contra un pool del store con `max_connections >= 4` (el diseño lo fija en 6); el `SELECT COALESCE(MAX(version),0)` de cada corredor (ACCESS SHARE, no bloqueado, ve 0) se completa, luego su `INSERT INTO events …` (ROW EXCLUSIVE) se bloquea en el bloqueo de tabla del holder; el test llama a `wait_until_blocked(observer, "%INSERT INTO events%", 4)` antes de liberar al holder; `holder.commit()`. Tras la liberación, exactamente un corredor confirma y los tres restantes reciben `23505` en `ux_events_identity_tenant`, descartan su transacción abortada, releen el stream **en una conexión de pool distinta**, y reportan `Conflict { expected: 0, actual: 1 }` (rastreable en `event_store.rs:198-247`).
  2. Comentario de documentación (textual): invariante — una carrera de append concurrente N-way sobre un stream deja exactamente un ganador, y cada uno de los N-1 perdedores reporta en su conflicto la versión real y ganadora, obtenida sólo después de que la propia transacción del store ya haya abortado por la violación de la restricción única y deba releer el stream en una conexión distinta; por qué no en proceso — esto exige forzar transacciones concurrentes reales a colisionar genuinamente en una restricción única real, más allá del punto de un abort de transacción real — un store simulado no tiene ningún abort por restricción única, y la rama de comprobación previa (ya cubierta por IS-1) no necesita ninguna carrera. Indica explícitamente, en el mismo comentario, que la cláusula de comprobación previa de SC-2 queda fuera del alcance de este test por diseño, no por omisión.
  3. VERDE: registrar el módulo; añadir la fila del README.
- **Evidencia:** exactamente 1 corredor tiene éxito; exactamente 3 corredores reportan `Conflict { expected: 0, actual: 1 }`, con `actual` obtenido de la ruta de relectura posterior al abort. `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` — bloque ≤1–2 min.
- **Nota D-8:** si este test revela que la relectura posterior a `23505` reporta un `actual` obsoleto o incorrecto (un defecto real en esa rama), se permite un arreglo pequeño y localizado **sólo** si es estrictamente necesario para satisfacer este invariante exacto, y **no debe** introducir una nueva API, un nuevo comportamiento contractual, un cambio arquitectónico ni una migración adicional. En caso contrario: una spec de seguimiento con nombre propio (D-8, OOS-7). Ningún arreglo está preautorizado por esta tarea.
- **Estado:** [x] Hecho — archivo creado con el fixture de corredores bajo bloqueo de tabla (`RaceEvent`, `race()`, `wait_until_blocked(test_pool, "%INSERT INTO events%", 4)`), módulo registrado en `tests/infrastructure.rs` con el comentario de alcance IS-2/IS-5, fila del README añadida a "The PostgreSQL concurrency invariants". Ningún defecto D-8 salió a la luz: la relectura posterior a `23505` reportó la versión ganadora correcta (`actual: 1`) en la primera ejecución con contenedor real, así que no hace falta arreglo ni spec de seguimiento. Ejecución con contenedor real: `events_identity_race_postgres::an_n_way_append_race_leaves_one_winner_and_reports_the_real_version_after_abort` pasó — exactamente 1 de 4 corredores ganó, exactamente 3 reportaron `Conflict { expected: 0, actual: 1 }`.

### T-03.2 — ROJO: IS-5, carrera comportamental de lógica trivaluada con tenant NULL (P1)

- **Archivo:** mismo archivo que T-03.1, `#[tokio::test]` adicional
- **Depende de:** T-03.1 (mismo archivo, un solo escritor)
- **Satisface:** IS-5, SC-5, spec.md "NULL-Tenant Stream Identity Honors SQL's Three-Valued Comparison Behaviorally"
- **Pasos:**
  1. ROJO: ejecutar la misma forma de carrera que T-03.1 con `tenant = None`, que carga `ux_events_identity_systemwide` (el índice parcial que existe únicamente porque `NULLS NOT DISTINCT` es de PostgreSQL 15+, según AD-3). Añadir: una inserción duplicada por SQL directo bajo un tenant NULL, con fallo esperado `23505` (demostrando que la identidad con tenant NULL NO está exenta de unicidad — segundo escenario MODIFICADO de spec.md); una inserción del mismo `(aggregate_type, aggregate_id, version)` bajo un tenant concreto, con éxito esperado (demostrando que los índices parciales systemwide y con-tenant no colisionan entre sí); y dos eventos bajo dos `aggregate_id` distintos, ambos con `tenant = None`, cada uno verificado como resuelto a su propio stream independiente sin colisión ni fusión falsa (escenario AÑADIDO de spec.md, "Two distinct systemwide-tenant streams resolve independently").
  2. Comentario de documentación (textual): invariante — la identidad de tenant `Option::None` se verifica comportamentalmente bajo la comparación trivaluada de SQL (`NULL = NULL` no es verdadero), no sólo desde el catálogo; dos streams systemwide nunca colisionan ni se fusionan silenciosamente, y la unicidad con tenant NULL se aplica de forma genuina; por qué no en proceso — `schema_index_assertion.rs` ya fija la forma del catálogo, pero sólo una inserción real contra una comparación NULL trivaluada real demuestra el comportamiento que implica.
- **Evidencia:** se observa `23505` para el duplicado con tenant NULL; la inserción con tenant concreto e idéntico `(aggregate_type, aggregate_id, version)` tiene éxito; los dos streams distintos con tenant NULL se listan de forma independiente vía `list_aggregate_ids` / `load()` sin contaminación cruzada.
- **Estado:** [x] Hecho — segundo `#[tokio::test]` añadido al mismo archivo (un solo escritor, misma fila de README que T-03.1, cubriendo ambos invariantes). Ejecución con contenedor real: `events_identity_race_postgres::null_tenant_identity_is_genuinely_unique_not_exempt_and_does_not_collide_with_a_concrete_tenant` pasó — exactamente 1 de 4 corredores ganó la carrera systemwide; el duplicado por SQL directo con tenant NULL falló con `23505` en `ux_events_identity_systemwide`; la inserción del mismo `(aggregate_type, aggregate_id, version)` bajo un tenant concreto tuvo éxito (sin colisión entre índices); los dos streams systemwide distintos de alpha/beta cargaron cada uno exactamente su propio evento sin contaminación. `cargo test --test ledger`: 9/9 pasaron (registro y anclaje satisfechos). Suite completa: 48 pasaron (46 preexistentes + 2 nuevos), 1 ignorado preexistente, 0 fallidos; suite terminó en 13.22s — bien dentro del presupuesto de bloque ≤1–2 min (SC-9). Contenedores reclamados (`docker ps -a` vacío para postgres).

---

## Grupo 4 — IS-4, P1 (peso completo según D-9): comportamiento transaccional del backfill de la migración 007

### T-04.1 — ROJO: `aggregate_type_backfill_postgres.rs`, C1 Aborted

- **Archivo:** nuevo `integration-tests/tests/infrastructure/aggregate_type_backfill_postgres.rs`
- **Depende de:** nada
- **Paralelo con:** T-01.*, T-02.*, T-03.*
- **Satisface:** IS-4 (caso C1), SC-4
- **Pasos:** sembrar una fila cuyo `aggregate_id` no coincida con ningún tipo registrado; tomar el pre-digest (`SELECT md5(string_agg(events::text, '|' ORDER BY id)) FROM events` — `events::text` renderiza cada columna como un literal compuesto, así que una columna añadida más adelante queda cubierta sin editar el test, según AD-5); ejecutar el backfill; verificar `Aborted(NoRegisteredTypeMatches)` y `rows_rewritten: 0`; tomar el post-digest; verificar igualdad de digests. **Nota, según la propia razón de AD-5:** este caso por sí solo demuestra el *orden* de las sentencias (`drop(tx)` se ejecuta antes de cualquier `UPDATE`, `aggregate_type_backfill.rs:269-288`), no el rollback transaccional — C2 abajo es lo que realmente demuestra la transacción.
- **Comentario de documentación (textual):** invariante — un abort antes del primer `UPDATE` del backfill deja la tabla byte a byte idéntica; por qué no en proceso — requiere una tabla real migrada y el orden real de sentencias del closure de abort.
- **Evidencia:** digest-antes == digest-después; se devuelve `Aborted(NoRegisteredTypeMatches)`.
- **Estado:** [x] Hecho — archivo creado; `c1_an_abort_before_any_write_leaves_the_table_byte_identical` siembra una fila `"orphan-123"` (ningún tipo registrado es prefijo), verifica `Aborted(NoRegisteredTypeMatches)`, `rows_rewritten: 0`, e igualdad de digests vía `SELECT md5(string_agg(events::text, '|' ORDER BY id))` (AD-5). Ejecución con contenedor real: pasó.

### T-04.2 — ROJO: C2 RolledBack

- **Archivo:** mismo archivo
- **Depende de:** T-04.1 (mismo archivo, un solo escritor)
- **Satisface:** IS-4 (caso C2) — necesario para que IS-4 demuestre una transacción genuina, según la razón de AD-5
- **Pasos:** sembrar un stream con las versiones 1 y 3 (un hueco, que divide limpiamente); ejecutar el backfill; verificar `RolledBack(StreamVersionsAreNotConsecutiveFromOne)`; tomar el post-digest y verificar igualdad con el pre-digest; verificar que `aggregate_type` sigue siendo **nullable** vía `information_schema.columns` (aún no `SET NOT NULL`) — éste es el único caso en el que realmente se escriben filas y luego se descartan, la propiedad que sólo una transacción real (no el orden de sentencias) puede tener.
- **Comentario de documentación (textual):** invariante — un rollback explícito tras al menos un `UPDATE` completado deja la tabla byte a byte idéntica, demostrando que el rollback — no meramente el orden de las sentencias — es lo que garantiza que no hay efecto parcial; por qué no en proceso — sólo un rollback de transacción real contra una tabla real migrada puede demostrar escrituras descartadas.
- **Evidencia:** digest-antes == digest-después; `information_schema.columns.is_nullable = 'YES'` para `aggregate_type` tras la ejecución.
- **Estado:** [x] Hecho — `c2_a_rollback_after_a_completed_write_leaves_the_table_byte_identical` siembra las versiones 1 y 3 del mismo stream (un hueco), verifica `RolledBack(StreamVersionsAreNotConsecutiveFromOne)`, igualdad de digests, y `aggregate_type` sigue con `is_nullable = 'YES'` tras la ejecución. No se reveló ningún defecto: el rollback se sostuvo en la primera ejecución con contenedor real, por lo que no se necesitó ningún arreglo D-8 ni spec de seguimiento. Ejecución con contenedor real: pasó.
- **Nota D-8:** éste es el caso con más probabilidad de revelar un defecto real (AD-5 lo nombra como la única prueba genuina de rollback transaccional que añade PROD-015). Si el rollback no se cumple, se permite un arreglo pequeño y localizado **sólo** si es estrictamente necesario para satisfacer este invariante exacto de IS-4, y no debe introducir una nueva API, un nuevo comportamiento contractual, un cambio arquitectónico ni una migración adicional. En caso contrario: una spec de seguimiento con nombre propio.

### T-04.3 — ROJO: C3 Zero-row commit

- **Archivo:** mismo archivo
- **Depende de:** T-04.2
- **Satisface:** IS-4 (caso C3), SC-4
- **Pasos:** vaciar `events`; ejecutar el backfill; verificar `Committed`, `rows_scanned: 0`; verificar `information_schema.columns.is_nullable = 'NO'` para `aggregate_type` tras la ejecución — demostrando que la ejecución confirmó `SET NOT NULL` (la última sentencia antes del commit), no meramente "confirmó nada" (el propio punto de AD-5: sin esta aserción, una ejecución que no confirme absolutamente nada seguiría satisfaciendo una lectura más laxa de "confirma limpiamente").
- **Comentario de documentación (textual):** invariante — una ejecución sobre cero filas elegibles confirma sin efectos secundarios, incluyendo el `SET NOT NULL` a nivel de esquema; por qué no en proceso — requiere una tabla real migrada y una lectura real del catálogo para distinguir "confirmó la sentencia prevista" de "no confirmó nada".
- **Evidencia:** `Committed`, `rows_scanned: 0`; `is_nullable = 'NO'` tras la ejecución.
- **Estado:** [x] Hecho — `c3_a_zero_row_commit_still_commits_the_schema_level_not_null` ejecuta el backfill sobre una tabla vacía, verifica `Committed`, `rows_scanned: 0`, e `is_nullable = 'NO'` tras la ejecución. Ejecución con contenedor real: pasó.

### T-04.4 — ROJO: C4 Revert round-trip

- **Archivo:** mismo archivo
- **Depende de:** T-04.3
- **Satisface:** IS-4 (caso C4), SC-4
- **Pasos:** sembrar filas elegibles → tomar pre-digest → ejecutar el backfill (verificar `Committed`) → llamar a `revert_aggregate_type_column` → tomar el post-revert digest; verificar que el post-revert digest == pre-digest; verificar que la columna `aggregate_type` ya no existe vía `information_schema.columns`.
- **Comentario de documentación (textual):** invariante — un revert vuelve exactamente al estado previo al backfill; por qué no en proceso — requiere las rutas reales de migración hacia adelante y hacia atrás contra una base de datos real y migrada.
- **Evidencia:** igualdad de digests; ausencia de columna confirmada.
- **Estado:** [x] Hecho, con una corrección a la fórmula del digest respecto al texto literal de esta tarea — `c4_a_revert_rejoins_exactly_the_state_that_preceded_the_backfill` siembra dos filas elegibles, verifica `Committed`, revierte, y compara digests. **Corrección:** el digest plano `events::text` (la fórmula de AD-5, reutilizada tal cual para C1–C3) no puede expresar "sin cambios" aquí — `revert_aggregate_type_column` elimina la columna `aggregate_type` por completo, por lo que un compuesto construido a partir de todas las columnas tiene un campo menos tras el revert del que tenía antes de que se ejecutara el backfill, y nunca compararía como igual sin importar la fidelidad del contenido. Se usó en su lugar: un digest de columnas explícitas que nombra todas las columnas excepto `aggregate_type` (`digest_excluding_aggregate_type`), que es la única fórmula bajo la cual "vuelve exactamente al estado previo al backfill" es expresable — la *existencia* de la columna, no su contenido, es de lo que trata este caso, y se verifica por separado vía `information_schema.columns`. Ejecución con contenedor real: pasó, confirmando que tanto la igualdad de digests como la ausencia de columna se cumplen.

### T-04.5 — VERDE: registrar módulo + fila del ledger del README

- **Archivos:** `integration-tests/tests/infrastructure.rs`, `integration-tests/README.md` (nueva categoría "Migration transactional behaviour")
- **Depende de:** T-04.1 hasta T-04.4
- **Evidencia:** `cargo test --manifest-path integration-tests/Cargo.toml --test ledger` pasa; `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` muestra los cuatro casos C1–C4 pasando.
- **Estado:** [x] Hecho — módulo registrado en `tests/infrastructure.rs`; nueva sección "Migration transactional behaviour" + cuatro filas (C1–C4) agregadas a `README.md`, todas citando el único archivo; bloque Budget actualizado (nueva categoría `Migration transactional behaviour ... 1`, `Total infrastructure tests` 19 → 20). El guard del ledger se confirmó ROJO primero (ambas aserciones fallaron, nombrando `aggregate_type_backfill_postgres`), luego VERDE: `cargo test --test ledger`: 9/9 pasaron. Ejecución con contenedor real: 52 pasaron (48 preexistentes + 4 nuevas), 1 ignorada preexistente, 0 fallidas — los cuatro casos C1–C4 pasaron en la primera ejecución con contenedor real; la suite terminó en 16.48s, holgadamente dentro del presupuesto de ≤1–2 min por porción (SC-9). Contenedores recuperados.

### T-04.6 — Prueba de mutación IS-8 para IS-4 (M2), P1

- **Archivos (mutados temporalmente y luego revertidos):** `crates/persistence/src/postgres/aggregate_type_backfill.rs`; **Adición al ledger:** tabla de mutaciones de `integration-tests/README.md`
- **Depende de:** T-04.5
- **Satisface:** IS-8 (escenario "neutralizar la atomicidad transaccional/de unidad-de-trabajo hace fallar el test correspondiente", spec.md), SC-8 — **satisfecho exactamente como está escrito** para esta mutación (AD-7: M2 y M3 satisfacen la cláusula literal de SC-8 "la suite preexistente permanece en verde"; sólo M1c no lo hace)
- **Pasos (receta de 7 pasos de AD-7):**
  1. `shasum -a 256 crates/persistence/src/postgres/aggregate_type_backfill.rs` — ANTES.
  2. En la rama `StreamVersionsAreNotConsecutiveFromOne`, cambiar `tx.rollback().await?` → `tx.commit().await?`.
  3. Ejecutar la suite.
  4. Registrar: esperado — C2 de `aggregate_type_backfill_postgres` **falla** en la comparación de digests; todos los demás tests permanecen en verde.
  5. `git checkout -- crates/persistence/src/postgres/aggregate_type_backfill.rs`.
  6. `shasum -a 256` — DEBE ser igual a ANTES.
  7. Reejecutar como control negativo: todo en verde.
- **Evidencia:** fila del ledger del README `M2` con el par SHA-256 y el mensaje exacto de la aserción que falla.
- **Estado:** [x] Hecho — SHA-256 ANTES = `3381e5cf4852c9d99e6a30f227c6d0c625efc6ef84e92abdf6ee32bbc7085fed` (== tras restaurar, confirmado igual). Ejecución mutada: `aggregate_type_backfill_postgres::c2_a_rollback_after_a_completed_write_leaves_the_table_byte_identical` falló exactamente como se predijo, en la comparación de digests; todos los demás tests, incluyendo `c1`/`c3`/`c4` del mismo archivo, permanecieron en verde — 51 pasaron, 1 falló, 1 ignorada. La cláusula literal de SC-8 "la suite preexistente permanece en verde" se satisface exactamente como está escrita para esta mutación (según AD-7, a diferencia de M1c). Restaurado vía `git checkout --`; hash reconfirmado igual; reejecución como control negativo: 52 pasaron, 0 fallaron, 1 ignorada. `git diff --stat` sobre `aggregate_type_backfill.rs` está vacío — ningún archivo de producción quedó modificado. Fila `M2` del README agregada.

---

## Grupo 5 — IS-9, P1: bloque acotado de compatibilidad PG14

### T-05.1 — ROJO: `pg14_compatibility.rs`, T0 guardia anti-vacuidad

- **Archivo:** nuevo `integration-tests/tests/infrastructure/pg14_compatibility.rs`
- **Depende de:** T-00.2 (`pg14_database()`, segundo contenedor)
- **Paralelo con:** T-01.* hasta T-04.*
- **Satisface:** IS-9 (aserción T0), SC-13
- **Pasos:** `db = pg14_database().await;` consultar `current_setting('server_version_num')::int` y verificar que está en `[140000, 150000)`. Ésta es la misma guardia de estilo "tres conjuntos vacíos" contra la vacuidad que ya tiene el guard del ledger: sin ella, una errata en la etiqueta del contenedor haría correr silenciosamente el "bloque PG14" contra PG16 y no demostraría nada.
- **Comentario de documentación (textual, cubre todo el archivo):** invariante — PG14 sigue siendo un piso de compatibilidad verificado y real, exactamente para los invariantes sensibles a la versión que se nombran abajo (T1–T3), nunca una segunda ejecución completa de la suite principal; por qué no en proceso — un piso de compatibilidad sólo puede demostrarse contra la versión real del motor objetivo.
- **Evidencia:** la aserción se cumple. (Esta guardia es un control de corrección sobre el propio contenedor del bloque, no una de las tres mutaciones IS-8 comprometidas — una mutación deliberada de la etiqueta del contenedor no forma parte del conjunto de mutaciones comprometido en este cambio.)
- **Estado:** [x] Hecho — archivo nuevo `integration-tests/tests/infrastructure/pg14_compatibility.rs`; `t0_the_pg14_container_genuinely_reports_a_pg14_server_version` consulta `current_setting('server_version_num')::int` contra `pg14_database()` y verifica que cae en `[140000, 150000)`. Ejecución con contenedor real: pasó.

### T-05.2 — ROJO: T1, el conjunto completo de migraciones 001–012 se aplica limpiamente en PG14

- **Archivo:** mismo archivo
- **Depende de:** T-05.1 (mismo archivo, un solo escritor)
- **Satisface:** IS-9 (aserción T1)
- **Pasos:** confirmar que `migrations::run()` in situ de `pg14_database()` (ya ejecutado por el fixture) se completó sin error; verificar que la tabla de seguimiento de migraciones registra las migraciones 001 a 012 aplicadas. Citar en el comentario de documentación por qué 008, 011 y 012 son las sensibles a la versión: `NULLS NOT DISTINCT` es de PostgreSQL 15+ y el piso es 14 (el propio comentario de 008 registra el error de sintaxis en la imagen 14); 012 además usa `DELETE … USING (SELECT … ROW_NUMBER() OVER …)`.
- **Evidencia:** sin error de migración; la tabla de seguimiento muestra 12 migraciones aplicadas.
- **Estado:** [x] Hecho, con una corrección respecto al texto literal de esta tarea — `crates/persistence/src/postgres/migrations.rs`'s `run()` no lleva ninguna tabla de seguimiento de migraciones: vuelve a aplicar el SQL idempotente (`IF NOT EXISTS`) de cada migración registrada en cada llamada, sin que nada registre qué nombres ya corrieron, así que no hay tabla de seguimiento que consultar. `t1_the_full_migration_set_applies_cleanly_and_leaves_every_version_sensitive_artifact_present` verifica en su lugar el equivalente positivo y verificable: cada artefacto de esquema sensible a la versión — los ocho índices únicos parciales de las migraciones 008/010/011/012, `events.aggregate_type` y `events.operation_key` de 007/009, y las tablas `operation_reservations`/`operation_receipts` de 010/011 — existe genuinamente en el destino PG14 vía `pg_indexes`/`information_schema`, después de que el `migrations::run()` in situ (no plantillado) de `pg14_database()` ya se completó sin entrar en pánico. Ejecución con contenedor real: pasó.

### T-05.3 — ROJO: T2, el duplicado bajo `tenant_id IS NULL` se rechaza con `23505` en PG14

- **Archivo:** mismo archivo
- **Depende de:** T-05.2
- **Satisface:** IS-9 (aserción T2)
- **Pasos:** inserción por SQL directo de un `(aggregate_type, aggregate_id, version)` duplicado bajo `tenant_id IS NULL`; verificar el código de error de PostgreSQL `23505`.
- **Evidencia:** se observa `23505`.
- **Estado:** [x] Hecho — `t2_a_systemwide_duplicate_identity_is_refused_with_23505_on_pg14` inserta una fila con `tenant_id IS NULL`, `aggregate_type = 'order'`, `aggregate_id = 'id-1'`, `version = 1`, y luego repite la inserción idéntica; verifica que la segunda devuelve `sqlx::Error::Database` con `.code() == Some("23505")`, ejercitando directamente `ux_events_identity_systemwide`. Ejecución con contenedor real: pasó.

### T-05.4 — ROJO: T3, commit del backfill de la migración 007 + round trip de revert en PG14

- **Archivo:** mismo archivo
- **Depende de:** T-05.3
- **Satisface:** IS-9 (aserción T3)
- **Pasos:** reflejar la forma de T-04.4 contra la base de datos PG14: sembrar → ejecutar `backfill_aggregate_type` (verificar `Committed`) → ejecutar `revert_aggregate_type_column` → verificar el round trip (digest o verificación de presencia de columna).
- **Evidencia:** `Committed`; round trip de revert confirmado.
- **Estado:** [x] Hecho — `t3_the_backfill_and_its_revert_round_trip_cleanly_on_pg14` siembra una fila `"user-7"` contra la base de datos PG14, toma el pre-digest de columnas explícitas (la fórmula corregida de T-04.4, reutilizada aquí porque el desajuste de forma por eliminación de columna de C4 aplica idénticamente en PG14), ejecuta `backfill_aggregate_type` (verifica `Committed`), llama a `revert_aggregate_type_column`, verifica igualdad de post-digest y ausencia de la columna `aggregate_type` vía `information_schema.columns`. Ejecución con contenedor real: pasó.

### T-05.5 — VERDE: registrar módulo + fila del ledger del README para IS-9

- **Archivos:** `integration-tests/tests/infrastructure.rs`, `integration-tests/README.md` (nueva categoría "Version-floor compatibility")
- **Depende de:** T-05.1 hasta T-05.4
- **Pasos:** registrar el módulo; añadir la fila del ledger; añadir una nota explícita en prosa (según la propia lista "Explicitly not on PG14" de AD-6) indicando que IS-1, IS-2, IS-3, IS-6, y los casos C1/C2 de abort/rollback de IS-4, además de los dieciséis tests preexistentes, **no** se ejecutan en PG14 — este archivo apunta exactamente a T0–T3, nada más.
- **Evidencia:** `cargo test --manifest-path integration-tests/Cargo.toml --test ledger` en verde; un `grep` del archivo muestra exactamente cuatro funciones `#[tokio::test]` (T0–T3), ninguna nombrada según contención, fencing o unidad de trabajo.
- **Estado:** [x] Hecho — módulo registrado en `tests/infrastructure.rs`; nueva sección "Version-floor compatibility" + cuatro filas (T0–T3) agregadas a `README.md`, todas citando el único archivo, con la nota explícita en prosa "no ejecutado en PG14" según AD-6; bloque Budget actualizado (nueva categoría `Version-floor compatibility ... 1`, `Total infrastructure tests` 20 → 21). El guard del ledger se confirmó ROJO primero (`every_test_on_disk_is_accounted_for_in_the_ledger` falló, nombrando `pg14_compatibility`), luego VERDE: `cargo test --test ledger`: 9/9 pasaron. `rg -c '#\[tokio::test\]' pg14_compatibility.rs` = 4, nombradas `t0_`/`t1_`/`t2_`/`t3_` — ninguna hace referencia a contención, fencing o unidad de trabajo. Ejecución con contenedor real: 56 pasaron (52 preexistentes + 4 nuevas), 1 ignorada preexistente, 0 fallidas — los cuatro casos T0–T3 pasaron en la primera ejecución con contenedor real; la suite terminó en 13.52s total (ambos contenedores), holgadamente dentro del presupuesto de ≤1–2 min por porción (SC-9). Contenedores recuperados.

---

## Grupo 6 — Pase final del ledger del README, verificación de presupuesto, comprobación de seguridad

### T-06.1 — Pase consolidado del ledger del README

- **Archivo:** `integration-tests/README.md`
- **Depende de:** T-01.3, T-02.1, T-03.1/T-03.2, T-04.5, T-05.5 (todos los registros de módulo entregados)
- **Pasos:** actualizar `Total infrastructure tests` al nuevo total (convención existente: contar archivos/filas tal como ya se rastrea, siguiendo la regla exacta de conteo que ya usa este documento — verificar contra el archivo antes de editar, no asumir la heurística +1-por-archivo); confirmar que existen las cinco filas de categoría nuevas; confirmar que existen las cinco filas de mutación (`M1a`, `M1b`, `M1c`, `M2`, `M3`); confirmar que está presente la nota en prosa sobre el retiro de IS-6 hacia IS-1.
- **Evidencia:** `cargo test --manifest-path integration-tests/Cargo.toml --test ledger` en verde sin desviación en ninguna dirección (archivo↔módulo↔ledger).
- **Estado:** [x] Hecho — verificado contra el archivo en vez de asumido: la regla de conteo que este documento ya usa es un incremento por **archivo** en disco (`fd . integration-tests/tests/infrastructure -e rs | wc -l` = 21, coincide con `Total infrastructure tests ... 21` ya registrado — no hizo falta editar, la heurística +1-por-archivo se cumplió en cada grupo de este cambio porque cada uno aterrizó exactamente un archivo nuevo). Confirmado presente: las cinco citas de archivo nuevas (`durable_store_conformance_postgres.rs`, `lease_contention_postgres.rs`, `events_identity_race_postgres.rs`, `aggregate_type_backfill_postgres.rs` ×4 filas, `pg14_compatibility.rs` ×4 filas); exactamente dos encabezados de categoría totalmente nuevos (`## Migration transactional behaviour`, `## Version-floor compatibility`) — la frase "dos categorías nuevas" de la tabla File Changes de `design.md`, distinta de la formulación "cinco filas de categoría nuevas" de esta tarea, que esta línea de Estado interpreta como cinco citas de fila nuevas, tres aterrizando en categorías preexistentes (Durable-adapter conformance; PostgreSQL concurrency invariants ×2) y dos fundando categorías nuevas; las cinco filas de mutación (`M1a`, `M1b`, `M1c`, `M2`, `M3`); la nota en prosa del retiro de IS-6 hacia IS-1 (`integration-tests/README.md:275`, "retirado hacia esta misma ejecución según D-4/AD-4"). `cargo test --test ledger`: 9/9 pasaron, sin desviación.

### T-06.2 — Verificación del presupuesto de la suite completa (SC-9)

- **Depende de:** todos los grupos anteriores
- **Pasos:** ejecutar `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` de principio a fin; capturar la propia línea de tiempos del runner.
- **Evidencia:** tiempo total de reloj ≤5 minutos; ningún bloque individual >1–2 minutos; el tiempo de compilación se reporta por separado del tiempo de ejecución (comportamiento ya existente del runner, confirmado que sigue siendo cierto con dos contenedores y cinco archivos nuevos).
- **Estado:** [x] Hecho — ejecución completa de principio a fin, con `DOCKER_HOST` exportado para colima. Línea de tiempos propia del runner: `provisioned in 8.10s (PG16) / 8.69s (PG14) · template migrated at 8.80s · suite finished at 13.63s · reclaimed at 13.85s`. El único bloque de infraestructura (56 pasaron, 0 fallidas, 1 ignorada) tardó 2.92s de ejecución de tests; el ciclo de vida completo del runner (ambos contenedores aprovisionados, plantilla migrada, suite ejecutada, ambos contenedores recuperados) terminó a los 13.85s — holgadamente dentro del presupuesto de ≤1–2 min por bloque individual. El tiempo total de reloj de la invocación externa `cargo run` (incluyendo una recompilación de dependencias desde cero tras el nuevo archivo `pg14_compatibility.rs`) fue de 19.27s, holgadamente dentro del presupuesto total de ≤5 min. El tiempo de compilación (build de dependencias + target, reportado por las propias líneas de build de cargo) queda visiblemente separado de la línea de tiempos de ejecución propia del runner — confirmado que sigue siendo cierto con dos contenedores y cinco archivos nuevos.

### T-06.3 — Comprobación de cumplimiento del skill de seguridad (`skills/security/SKILL.md` Reglas 1 y 2)

- **Depende de:** los cinco archivos de test nuevos entregados
- **Pasos:** escanear cada llamada `sqlx::query` / `sqlx::query_scalar` en los cinco archivos nuevos y en las dos funciones nuevas de `src/lib.rs`; confirmar que ninguna interpolación o concatenación de cadenas llega al texto SQL — incluido el argumento `statement_like` de `wait_until_blocked`, que siempre es un fragmento literal codificado, pasado como `$1` vinculado, nunca formateado dentro de la cadena de la consulta; confirmar que toda consulta con alcance de tenant (las inserciones de IS-2/IS-5, las llamadas de conformidad de IS-1) vincula `tenant_id` como parámetro, nunca derivado sin vincular.
- **Evidencia:** informar PASS/BLOCK según el propio contrato de salida del skill de seguridad; se requieren cero hallazgos BLOCK antes de que este cambio pueda considerarse terminado.
- **Estado:** [x] Hecho — WARN, cero BLOCK. Se escaneó cada llamada `sqlx::query`/`sqlx::query_scalar`/`sqlx::query_as` en los cinco archivos nuevos (`durable_store_conformance_postgres.rs`, `lease_contention_postgres.rs`, `events_identity_race_postgres.rs`, `aggregate_type_backfill_postgres.rs`, `pg14_compatibility.rs`) y en las dos funciones nuevas de `integration-tests/src/lib.rs` (`wait_until_blocked`, `pg14_database`). Regla 1 (sin interpolación/concatenación de entrada de usuario, variables o datos externos en el texto SQL): cero violaciones — todo valor dinámico (`joined_aggregate_id`, `tenant_id`, `version`, `index_name`, `table_name`, `column_name`, `KEY`, `statement_like`, `AGGREGATE_TYPE`, `occurred_at`, etc.) se pasa mediante `.bind()`; se confirmó que `statement_like` de `wait_until_blocked` es siempre un fragmento literal codificado en sus dos puntos de llamada, vinculado como `$1`, nunca formateado en la cadena. El `format!("CREATE DATABASE {name}")` de `pg14_database()` interpola un identificador de base de datos, no un dato — `name` se construye únicamente a partir de un prefijo literal fijo más un contador interno `AtomicU64`, nunca entrada externa o de usuario, siguiendo el mismo patrón preexistente de `isolated_database()` en el mismo archivo; los identificadores DDL no pueden vincularse en el protocolo de PostgreSQL, y la generación cerrada y controlada por código es el equivalente de la lista permitida que exige la Regla 1 para el caso no vinculable. WARN (advertencia, no BLOCK) sobre la letra de la Regla 2: tres bloques de texto SQL literal escriben `tenant_id` como `NULL` codificado en lugar de vincularlo — `events_identity_race_postgres.rs:246` y `pg14_compatibility.rs:152,160` (cada uno prueba la partición NULL-tenant/unicidad sistémica, que es el sujeto de prueba deliberado y fijo, no derivado de ninguna variable, entrada de usuario o fuente externa); `pg14_compatibility.rs:190` escribe de igual forma `'tenant-a'` como literal. Riesgo real nulo (ninguna variable llega jamás a estas cadenas de consulta, por lo que la Regla 1 no está implicada), pero la letra de la Regla 2 ("DEBE incluir tenant_id como parámetro vinculado") no se cumple con un literal codificado — corrección recomendada, no requerida antes de la fusión: cambiar estos cuatro literales a llamadas `.bind()` por consistencia con el resto de la suite. Cero hallazgos BLOCK.

### T-06.4 — Plan de fraccionamiento de PRs (R-7), sólo documentación

- **Depende de:** nada (puede escribirse en cualquier momento, informa el orden de fusión)
- **Pasos:** registrar el orden de fraccionamiento planeado para que ningún PR individual exceda el presupuesto de revisión de ~400 líneas:
  - Bloque 1 — T-00.1, T-01.1–T-01.4 (IS-1/IS-6, cierra por sí solo AC10 de #275; un PR completo por sí mismo según la nota de Migración/Rollout de `design.md`).
  - Bloque 2 — T-02.1–T-02.2 (IS-3 + su prueba de mutación).
  - Bloque 3 — T-03.1–T-03.2 (IS-2/IS-5).
  - Bloque 4 — T-04.1–T-04.6 (IS-4 + M2).
  - Bloque 5 — T-00.2, T-05.1–T-05.5 (IS-9/PG14).
  - Bloque 6 — T-06.1–T-06.4 (pase final de ledger/presupuesto/seguridad, si no se pliega ya en el bloque 5).
- **Evidencia:** el tamaño del diff de cada bloque se estima a partir de la tabla de cambios de archivos de `design.md` antes de abrir el PR; cualquier bloque que tienda a superar el presupuesto se fracciona aún más, nunca se fusiona para elevar el presupuesto (refleja la propia regla de R-2 para el tiempo de reloj).
- **Estado:** [x] Hecho — el orden de seis bloques anterior es el plan registrado (solo documentación, sin cambio de código). Verificación de tamaño de diff contra la tabla de cambios de archivos de `design.md`: el Bloque 1 (T-00.1/T-01.1–T-01.4) toca la migración + `durable_store_conformance_postgres.rs` + el andamiaje de `lib.rs`; el Bloque 2 (T-02.1–T-02.2) añade un archivo (`lease_contention_postgres.rs`) + `wait_until_blocked`; el Bloque 3 (T-03.1–T-03.2) añade un archivo (`events_identity_race_postgres.rs`); el Bloque 4 (T-04.1–T-04.6) añade `aggregate_type_backfill_postgres.rs`, ejerciendo el módulo preexistente `aggregate_type_backfill.rs` (de PROD-012, sin modificar salvo como objetivo de la prueba de mutación M2) (el bloque más grande, aun así un solo archivo nuevo); el Bloque 5 (T-00.2/T-05.1–T-05.5) añade `pg14_compatibility.rs` + `pg14_database`; el Bloque 6 (T-06.1–T-06.4) es solo prosa de README/ledger. Cada bloque aterriza exactamente un archivo de test nuevo (o ninguno, en el Bloque 6), manteniendo cada bloque muy por debajo del presupuesto de ~400 líneas — ningún bloque necesita fraccionarse más.

---

## Grupo 7 — Mapeo AC por AC del issue #275 (obligación D-7, sólo documentación)

**Reconciliado contra el texto textual del issue.** El cuerpo real del issue #275 ha sido
obtenido y lleva exactamente 13 criterios de aceptación formales. La tabla de abajo es el mapeo
AC por AC verificado — no una suposición, no un parafraseo pendiente de confirmación.

| AC | Disposición | Evidencia |
|---|---|---|
| AC1 | Andamiaje preexistente, no tocado por este cambio | `integration-tests/Cargo.toml` |
| AC2 | Preexistente, no afectado por este cambio (la lista de miembros del workspace raíz no cambia) | lista de miembros del `Cargo.toml` raíz |
| AC3 | Preexistente; extendido (no establecido) por este cambio | `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` |
| AC4 | Se cree resuelto por PROD-012/PROD-012A (según `proposal.md`); trabajo de script de guard, fuera del alcance de este cambio — no es un invariante de PostgreSQL | requiere verificación contra `scripts/detect-integration-tests.sh` y la propia evidencia de cierre de PROD-012/PROD-012A, no producida por este cambio |
| AC5 | Se cree resuelto por PROD-012/PROD-012A; fuera del alcance de este cambio, mismo razonamiento que AC4 | igual que AC4 |
| AC6 | Preexistente (un único contenedor PG16 compartido en `main.rs`). El Grupo 0 de este cambio (T-00.2) añade un segundo contenedor compartido, distinto, sólo para compatibilidad con PG14 — no un contenedor por test, y no una desviación de "un único PostgreSQL compartido por ejecución" para la suite principal | `integration-tests/src/main.rs`; evidencia de conteo de contenedores de T-00.2 |
| AC7 | Directamente satisfecho por el propio requisito de proceso de este cambio — cada archivo de test nuevo lleva el invariante/justificación textual del comentario de documentación que exige `tasks.md` | el paso "Comentario de documentación (textual)" de cada tarea |
| AC8 | Directamente satisfecho — T-03.1 acota explícitamente IS-2 sólo a la ruta de relectura post-`23505` (la comprobación previa ya está cubierta por la ejecución de conformidad de IS-1, según la regla de admisión 3); IS-6 se retira hacia IS-1 (T-01.1/T-01.2) en vez de duplicarse | campo "Satisface" de T-03.1; nota de retiro de IS-6 de T-01.1 |
| AC9 | Directamente satisfecho — `wait_until_blocked` (T-00.1) es la primitiva compartida de sondeo-con-plazo que usa cada test de contención de este cambio | T-00.1; su uso en T-02.1, T-03.1 |
| AC10 | Cerrado por este cambio | T-01.1, T-01.2 |
| AC11 | Ya satisfecho antes de PROD-015 por el `fencing_window_postgres.rs` existente de un solo contendiente (referenciado, no creado, por T-00.1). Este cambio no cierra AC11 — extiende la garantía a seis contendientes reales (IS-3/T-02.1), una propiedad más fuerte que la que exige literalmente el AC | `integration-tests/tests/infrastructure/fencing_window_postgres.rs` (preexistente); T-02.1 (extensión) |
| AC12 | Mecanismo de medición preexistente; este cambio no debe hacerlo retroceder | T-06.2 |
| AC13 | Criterio de proceso, no un test — satisfecho por el propio enfoque de este documento centrado en invariantes (no se declara ningún objetivo de número de tests en ninguna parte de `tasks.md`) | la propia estructura de este documento |

**Referencias cruzadas al repositorio confirmadas para las disposiciones anteriores:** `integration-tests/tests/infrastructure/fencing_window_postgres.rs` es anterior a este cambio (entregado bajo PROD-012, commit `5f085d1`); `integration-tests/src/main.rs` aprovisiona exactamente un contenedor `Postgres::default().with_tag("16")` antes de que T-00.2 de este cambio lo extienda (AC6, AC11).

**División recomendada (D-7, sólo documentación — ejecutarla queda explícitamente fuera de este
cambio SDD, según la propuesta y según esta tarea):**
1. Marcar en #275 AC7, AC8, AC9, AC10 y AC11 como satisfechos por este cambio, con enlaces a los archivos que los entregan (T-01.1/T-01.2 para AC10; T-00.1 para AC7/AC9; T-03.1 para AC8; T-02.1 para la extensión de AC11).
2. Verificar AC4 y AC5 por separado contra la propia evidencia de cierre de PROD-012/PROD-012A — este cambio no produce esa evidencia y no debe reclamarla.
3. Una futura spec llamada PROD-016 se hará cargo de la verificación de HTTP / socket / OTLP, si algún alcance restante de las secciones descriptivas de #275 lo justifica.

Ningún issue se crea, edita ni cierra por esta tarea ni por este cambio.

---

## Autochequeo de las siete puertas

| # | Puerta | Estado | Dónde se aplica |
|---|---|---|---|
| 1 | Tareas atómicas, sin mezclar HTTP/OTLP/otros transportes | **Satisfecha.** Cada tarea de arriba se corresponde con exactamente un invariante de PostgreSQL (IS-1 a IS-9, o un prerrequisito compartido para uno de ellos). Ninguna tarea toca `crates/transport`, la ruta OTLP de `crates/infrastructure`, ni la ruta HTTP de `examples/reference-app`; sólo se nombran en la tabla de mapeo del Grupo 7 como explícitamente fuera de alcance | Campo "Satisface" de cada tarea; tabla del Grupo 7 |
| 2 | PG16 como suite principal | **Satisfecha.** Los Grupos 1–4 (IS-1, IS-2, IS-3, IS-4, IS-6) apuntan todos al contenedor PG16 compartido por ejecución ya existente vía `isolated_database()` — ninguna tarea de esos grupos toca el contenedor PG14 | T-01.*, T-02.*, T-03.*, T-04.* usan todos `db = isolated_database()` contra la plantilla PG16 |
| 3 | PG14 como un bloque de compatibilidad específico, nunca una segunda ejecución completa | **Satisfecha.** El Grupo 5 son exactamente cuatro aserciones (T0–T3) en un único archivo, acotadas a la migración 007 y a la característica del catálogo `NULLS NOT DISTINCT` sensible a la versión, vía el segundo contenedor que aprovisiona T-00.2 (AD-6). T-05.5 registra explícitamente qué *no* se ejecuta en PG14 | T-05.1–T-05.5; la nota "Explicitly not on PG14" de T-05.5 |
| 4 | Reutilizar `ego-testkit`, nunca duplicar conformidad | **Satisfecha.** T-01.1/T-01.2 llaman directamente a `assert_event_store_conformance` y `assert_reservation_store_conformance` contra los adaptadores durables, con sólo un tipo fixture local (`ConformanceEvent`), nunca un conjunto de aserciones re-derivado | T-01.1, T-01.2 |
| 5 | Contención/fencing real como P0 | **Satisfecha.** El Grupo 2 (IS-3 + M1a/M1b/M1c) y T-03.1 del Grupo 3 (IS-2, sólo alcance post-`23505`) están ambos marcados explícitamente como P0 | Leyenda de prioridad; cabeceras del Grupo 2/Grupo 3 |
| 6 | D-8 respetado ante cualquier defecto expuesto | **Satisfecha.** Toda tarea que toca `aggregate_type_backfill.rs` (T-04.2 en concreto) o la ruta de manejo de conflictos de la tabla `events` (T-03.1) lleva una nota D-8 explícita: arreglar sólo si es estrictamente necesario para el invariante exacto, sin nueva API/comportamiento contractual/arquitectura/migración, si no, una spec de seguimiento con nombre propio. Ninguna tarea preautoriza un arreglo | Campos "Nota D-8" de T-01.3, T-03.1, T-04.2 |
| 7 | Evidencia verificable por tarea, paridad ES/EN | **Satisfecha.** Cada tarea de arriba lleva un campo "Evidencia" concreto — un comando, una aserción o un valor a capturar, nunca un simple "el test pasa". `tasks.es.md` refleja este archivo 1:1: mismos IDs de tarea, mismo orden, misma evidencia, traducida fielmente | Campo "Evidencia" de cada tarea; `tasks.es.md` |

**Excepción, declarada en vez de ocultada:** el mapeo AC por AC de #275 en el Grupo 7 ha sido
reconciliado por completo contra el texto textual del issue — ya no es un vacío. Las puertas 1–6
siguen siendo autocontenidas dentro de los propios archivos de este cambio y no se vieron
afectadas por la reconciliación.
