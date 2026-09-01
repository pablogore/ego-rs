# Diseño: PROD-015 — Verificación de Integración con PostgreSQL Real

> Compañero de revisión en español. Fuente canónica: `design.md` (identificadores 1:1).

## Enfoque Técnico

Cinco archivos nuevos bajo `integration-tests/tests/infrastructure/`, cada uno con su registro
de módulo y su fila en el ledger del README. Sin arnés nuevo, sin mecanismo de aislamiento
nuevo, sin runner nuevo: la suite ya aprovisiona un PostgreSQL por ejecución, clona una
plantilla ya migrada en una base de datos por test, y prohíbe dormir. Este cambio agrega
puntos de llamada y un único helper de sincronización compartido, más una extensión acotada
del runner para el slice de PG14 (IS-9).

Nombres corregidos contra HEAD: el event store durable es `PostgreSQLEventStore<E, F>`
(`crates/persistence/src/postgres/event_store.rs:53`), no `PostgresEventStore` como lo
escribe la propuesta. `PostgresOperationReservationStore` sí es correcto tal como está.

| Archivo | Invariantes | Categoría |
|---------|------------|-----------|
| `durable_store_conformance_postgres.rs` | IS-1, IS-6 | Conformidad del adaptador durable |
| `events_identity_race_postgres.rs` | IS-2, IS-5 | Invariantes de concurrencia de PostgreSQL |
| `lease_contention_postgres.rs` | IS-3 | Invariantes de concurrencia de PostgreSQL |
| `aggregate_type_backfill_postgres.rs` | IS-4 | Comportamiento transaccional de migración |
| `pg14_compatibility.rs` | IS-9 | Compatibilidad del piso de versión |

## Decisiones de Arquitectura

### AD-1 — Aprovisionamiento y compartición: un contenedor PG16 propiedad del runner, sin cambios

**Elección.** `integration-tests/src/main.rs` ya es dueño de todo el ciclo de vida y sigue
siéndolo. Orden por ejecución, sin cambios para los cinco archivos nuevos:

```
run-suite (cargo run --bin run-suite)
  1. cargo test --test ledger --no-run      hermético; un fallo de compilación es Unavailable, no Diverged
  2. cargo test --test ledger               deriva = exit 101, nada aprovisionado
  3. Postgres::default().with_tag("16").start()
  4. CREATE DATABASE ego_template; migrations::run(&template); cerrar ambos pools
  5. cargo test --test infrastructure       hijo, al que solo se le dice EGO_IT_PG_HOST / EGO_IT_PG_PORT
  6. container.rm().await                   dentro de un runtime vivo, en todo camino
  7. salir exactamente con el código de la suite
```

Los binarios de test que lo comparten son exactamente uno: `tests/infrastructure`. Los cinco
archivos nuevos son módulos dentro de ese único target (`tests/infrastructure.rs`), así que
comparten el contenedor por construcción, no por convención. `tests/ledger` es un segundo
target y no comparte nada — es hermético a propósito.

**Alternativas consideradas.** Un contenedor por archivo (la forma de la que se reconstruyó
la suite); un `OnceCell` de proceso sosteniendo el contenedor dentro del binario de test.

**Fundamento.** La forma con `OnceCell` fue medida y filtró tres contenedores en tres
ejecuciones: libtest no tiene teardown a nivel de suite, así que el `Drop` asíncrono corre al
salir del proceso sin runtime que lo ejecute. Por eso existe el runner, y aquí nada de eso
cambia.

### AD-2 — Aislamiento: una base de datos por test, clonada de la plantilla. Sin cambios.

**Elección.** Cada `#[tokio::test]` llama a `ego_integration_tests::isolated_database()`,
obtiene `ego_test_{n}` clonada de `ego_template` (migrada una sola vez, en el paso 4 de
arriba), y llama a `db.close()` al final. La concurrencia está acotada por un semáforo en
`MAX_LIVE_DATABASES = 8`, que acota conexiones en lugar de serializar la suite.

**Alternativas consideradas.** Un esquema por test con disciplina de `search_path`.

**Fundamento.** Ya fue rechazada, por una razón de la que PROD-015 depende: varios tests aquí
escanean tablas enteras sin `WHERE`, y dentro de su propia base de datos esa consulta es
*correcta*. Este cambio agrega tres más de ese tipo — `SELECT count(*) FROM events`, el
digest de tabla completa del backfill, y el `purge_completed_before` del arnés de
reservaciones — así que el aislamiento por esquema obligaría a reescribir cada uno para que
sobreviva a su arnés. También resuelve **R-3** directamente:
`assert_event_store_conformance` afirma listados exactos de `list_aggregate_ids`, lo cual
solo tiene sentido en una base de datos que ningún vecino puede alcanzar.

Dos restricciones de dimensionamiento que este diseño fija, porque el valor por defecto no
alcanza:

- El pool del store del test de conformidad DEBE tener `max_connections >= 2` (el diseño usa
  4). Ver AD-4.
- El pool del store del test de IS-3 DEBE tener `max_connections >= 6` (el diseño usa 8) y el
  de IS-2 `>= 4` (el diseño usa 6). Los pools abiertos directamente desde `db.url()` en lugar
  de a través de `db.pool()` los cierra el propio test antes de `db.close()`, como exige
  `src/lib.rs`.

### AD-3 — Determinismo: un helper compartido `wait_until_blocked`, promovido a `src/lib.rs`

**Elección.** Extraer el `wait_until_contender_is_blocked` privado de
`fencing_window_postgres.rs` a
`ego_integration_tests::wait_until_blocked(observer, statement_like, expected)`. Sondea con
un plazo explícito y **falla el test** al vencerlo; el sleep interior es un intervalo de
sondeo, nunca un timeout que reemplace a una condición.

```sql
SELECT count(DISTINCT pid) FROM pg_stat_activity
WHERE wait_event_type = 'Lock'
  AND state = 'active'
  AND datname = current_database()     -- NUEVO, ver abajo
  AND query ILIKE $1
  AND pid <> pg_backend_pid()
```

Dos correcciones sobre la copia existente, ambas portantes:

1. **Se agrega `datname = current_database()`.** `pg_stat_activity` es a nivel de clúster, y
   hay hasta ocho bases de datos aisladas vivas a la vez. Hoy solo un test se bloquea sobre
   `operation_reservations`, así que la omisión es latente; IS-3 agrega un segundo, y sin
   este predicado cualquiera de los dos podría quedar satisfecho por el backend del otro y
   pasar sin haber forzado nada. Es un defecto de los fixtures de la propia suite, no de
   código de producción, así que D-8 no aplica.
2. **El fragmento de sentencia se estrecha de un nombre de tabla a una sentencia.** IS-3
   busca `'%UPDATE operation_reservations%'`, no `'%operation_reservations%'`. Eso es lo que
   convierte el conteo en *evidencia*: un contendiente contado aquí ya pasó su
   `INSERT … ON CONFLICT DO NOTHING` y su `SELECT`, así que llegar a `expected = 6` prueba
   que los seis leyeron la fila expirada antes de que ninguno escribiera.

**IS-3, seis contendientes, orquestados en vez de librados al azar.**

```
holder (tx de test_pool):  SELECT owner_id … WHERE operation_key = $1 FOR UPDATE   -- lock de fila sostenido
6 × contendiente (task):   store.reserve(owner-b_i, …)
                             INSERT … ON CONFLICT DO NOTHING   -> 0 filas, no espera
                             SELECT … (plano, MVCC)            -> ve token T, lease expirado
                             UPDATE … WHERE fencing_token = T
                                       AND lease_until <= now  -> SE BLOQUEA en el lock de fila
test:                      wait_until_blocked(observer, '%UPDATE operation_reservations%', 6)
holder:                    COMMIT                              -- libera, no cambia nada
6 × contendiente:          el UPDATE avanza; el CAS admite exactamente uno
```

Afirmaciones: exactamente un `TakenOver`; cinco `OtherInProgress`; `fencing_token = T + 1`,
nunca `T + 6`; `owner_id` es el del único ganador. Sin el sondeo el test sería
probabilístico, porque seis contendientes podrían resolverse en serie — el primero ganando y
los otros cinco leyendo un lease ya renovado y devolviendo `OtherInProgress` sin contender
jamás. Las afirmaciones igual pasarían y no probarían nada. El plazo convierte eso en un
fallo ruidoso en vez de una corrida verde.

`INSERT … ON CONFLICT DO NOTHING` es precisamente por qué la primera sentencia no se bloquea:
`DO NOTHING` no toma lock sobre una fila conflictiva ya *commiteada* ni espera por ella, a
diferencia de `DO UPDATE`. El diseño no descansa en que esa lectura sea correcta — si fuera
errónea, el fragmento de sentencia estrechado hace que el sondeo nunca llegue a 6 y el plazo
falle el test por nombre, en vez de pasar sobre una ventana nunca forzada.

**IS-2/IS-5, carrera de append N-way**, la misma forma con un lock de tabla en lugar de uno
de fila:

```
holder:        BEGIN; LOCK TABLE events IN EXCLUSIVE MODE;   -- bloquea INSERT, permite SELECT plano
4 × corredor:  store.append(type, id, tenant, 0, [event])
                 SELECT COALESCE(MAX(version),0)  -- ACCESS SHARE, no bloqueado, ve 0
                 INSERT INTO events …             -- ROW EXCLUSIVE, SE BLOQUEA
test:          wait_until_blocked(observer, '%INSERT INTO events%', 4)
holder:        COMMIT
```

`EXCLUSIVE` entra en conflicto con `ROW EXCLUSIVE` y no con `ACCESS SHARE`, que es
exactamente la ventana que esto necesita. Tras la liberación, un corredor commitea y los
otros tres reciben `23505` sobre `ux_events_identity_tenant`, descartan su transacción
abortada, releen el stream **en otra conexión del pool**, y reportan
`Conflict { expected: 0, actual: 1 }` (`event_store.rs:198-247`). IS-5 es la misma carrera
con `tenant = None`, que carga `ux_events_identity_systemwide` — el índice parcial que existe
solo porque `NULLS NOT DISTINCT` es de PostgreSQL 15+ — más un insert duplicado por SQL
directo bajo tenant NULL que debe fallar con `23505`, y un insert del mismo
`(aggregate_type, aggregate_id, version)` bajo un tenant concreto que debe tener éxito.

**Corrección de alcance, y es real.** La primera cláusula de SC-2 — "una versión esperada
obsoleta produce un conflicto que reporta la versión actual real" — ya está satisfecha por
IS-1: el arnés compartido afirma exactamente eso
(`crates/testkit/src/event_store.rs:124-149`) y recorre la rama de *pre-chequeo* de `append`,
que no necesita carrera. Reafirmarlo aquí violaría la regla de admisión 3. Lo que ningún test
existente alcanza es la **relectura post-`23505`**: la rama donde la transacción ya está
abortada y `actual` debe venir de otra conexión. IS-2 se acota solo a esa rama. Marcado para
revisión cruzada con spec.

### AD-4 — Reutilización de IS-1, y el veredicto de D-4: **confirmado, IS-6 no necesita test propio**

**Elección.** Un archivo, dos funciones `#[tokio::test]`, ambas contra las mismas
definiciones compartidas:

```rust
// event store
let pool = connect(db.url(), 4).await;                       // >= 2 es obligatorio, ver abajo
let mut store = PostgreSQLEventStore::open(pool, deserialize).await?;
assert_event_store_conformance(&mut store, |kind| ConformanceEvent { … }).await;

// reservation store — la fábrica debe ser `Copy`, así que captura `&PgPool`, nunca un PgPool
let pool = &connect(db.url(), 4).await;
assert_reservation_store_conformance(|| async move {
    sqlx::query("TRUNCATE operation_reservations").execute(pool).await.unwrap();
    let clock = Arc::new(TestClock::new(epoch()));
    (PostgresOperationReservationStore::new(pool.clone(), clock.clone()), clock)
}).await;
```

Tres restricciones concretas que imponen las firmas, verificadas en vez de asumidas:

- `assert_reservation_store_conformance<S, F, Fut>(fresh: F) where F: Fn() -> Fut + Copy`
  (`reservation_conformance.rs:963`). `Copy` prohíbe capturar un `PgPool` propio; una
  referencia compartida sí es `Copy`, así que la clausura captura `&PgPool`. La forma que
  declara D-5 (`Fn() -> (S, Arc<TestClock>)`) es cercana pero no exacta — la fábrica es
  **asíncrona**.
- `fresh()` se llama **21 veces**, y el grupo de purga afirma conteos de tabla completa
  (`removed == 0` en `:937`), así que una fábrica que no reseteara arrastraría filas de
  escenarios anteriores hacia afirmaciones posteriores. `TRUNCATE operation_reservations` es
  el reseteo: este store es dueño de exactamente una tabla, así que truncarla es suficiente y
  barato. Una base de datos aislada nueva por llamada fue rechazada — 21 `CREATE DATABASE`
  serializados contra una plantilla, sin aislamiento adicional alguno.
- Cada archivo nuevo define su propio `ConformanceEvent` local. Eso es un fixture, no un
  contrato re-derivado; la copia de `crates/infrastructure` es privada del target de test de
  otro crate y no puede importarse. El **arnés** es compartido, que es lo que IS-1 exige.

**Veredicto de D-4: la suposición se sostiene.** Trazado en HEAD:
`PostgreSQLEventStore::begin()` llama a `self.pool.begin()` (`event_store.rs:414-424`), que
toma su propia conexión del pool y la retiene durante toda la vida de la unidad de trabajo;
`PostgreSQLEventStore::load()` llama a `.fetch_all(&self.pool)` (`event_store.rs:274`), que
toma una conexión **distinta**. El "un append no commiteado no debe ser visible a un lector"
del arnés (`event_store.rs:356-363`) y la afirmación correspondiente sobre el receipt
(`:493-501`) se vuelven, por tanto, afirmaciones genuinas de aislamiento `READ COMMITTED`
entre conexiones contra PostgreSQL real, gratis. **IS-6 no obtiene test propio ni fila de
ledger propia.**

La afirmación es bidireccional por construcción, así que no puede pasar de forma vacía: si el
lector de algún modo compartiera la conexión del escritor, *vería* las filas en staging y la
afirmación pre-commit fallaría; la afirmación post-commit exige después que esas mismas filas
aparezcan. Un mecanismo, ambas direcciones.

**La restricción `max_connections >= 2` es portante, no higiene.** Con un pool de uno,
`load()` esperaría por la conexión que retiene la unidad de trabajo abierta y fallaría como
timeout de pool — un test de aislamiento que falla por una razón ajena al aislamiento. Fijado
en 4.

**Nota sobre D-8.** Ningún camino de este diseño preautoriza un arreglo. El foco de defecto
más probable es la rama de fingerprint conflictivo de
`PostgresEventStoreUnitOfWork::confirm_receipt`, que el arnés exige que sea `Conflict` y que
nunca se ha ejecutado contra PostgreSQL real. Un defecto ahí lo encuentra IS-1, y la
excepción de arreglo pequeño de D-8 cubre solo IS-2 e IS-4 — así que se convierte en un spec
de seguimiento nombrado, no en alcance absorbido.

### AD-5 — IS-4: la garantía transaccional es el rollback, no el abort

**Elección.** Cuatro casos en un archivo, cada uno comparando un digest de tabla completa
tomado antes y después:

```sql
SELECT md5(string_agg(events::text, '|' ORDER BY id)) FROM events
```

`events::text` renderiza la fila entera como literal compuesto, así que toda columna queda
incluida y una columna agregada después queda cubierta sin editar el test — la misma idea de
exhaustividad estructural que usa el arnés cuando desestructura `StoredEvent` sin `..`.

| Caso | Preparación | Esperado |
|---|---|---|
| C1 Aborted | una fila cuyo `aggregate_id` no coincide con ningún tipo registrado | `Aborted(NoRegisteredTypeMatches)`, `rows_rewritten: 0`, digest sin cambios |
| C2 RolledBack | un stream con versiones 1 y 3 (un hueco), que separa limpiamente | `RolledBack(StreamVersionsAreNotConsecutiveFromOne)`, digest sin cambios, `aggregate_type` **sigue siendo nullable** |
| C3 Cero filas | `events` vacía | `Committed`, `rows_scanned: 0`, y `information_schema.columns` ahora reporta `is_nullable = 'NO'` |
| C4 Revert | sembrar → digest → backfill (`Committed`) → `revert_aggregate_type_column` | digest idéntico al previo al backfill, columna eliminada |

**Fundamento, y cambia lo que IS-4 debe contener.** La garantía de C1 viene del *orden*, no
de la transacción: la clausura de abort de `backfill_aggregate_type` ejecuta `drop(tx)` antes
de emitir cualquier `UPDATE` (`aggregate_type_backfill.rs:269-288`), y el propio código lo
dice. Así que un test de C1 solo se describiría como probando atomicidad transaccional
mientras prueba orden de sentencias. C2 es el único caso donde filas se escriben genuinamente
y luego se descartan, que es la propiedad que solo una transacción real puede tener y solo un
PostgreSQL real puede demostrar. Por tanto C2 es obligatorio, y la redacción de IS-4 en la
propuesta (abort / cero filas / revert) no lo nombra. Marcado para revisión cruzada con spec.

La afirmación `is_nullable` de C3 importa porque `SET NOT NULL` es la última sentencia antes
del commit; sin ella, "una corrida de cero filas commitea" queda satisfecha por una corrida
que no commitea nada.

### AD-6 — IS-9: un segundo contenedor, fijado por versión, propiedad del mismo runner

**Elección.** El runner arranca un **segundo** contenedor,
`Postgres::default().with_tag("14")`, publica `EGO_IT_PG14_HOST` / `EGO_IT_PG14_PORT`, y
reclama **ambos** en todo camino de salida antes de devolver el código de la suite.
`src/lib.rs` gana `pg14_database() -> IsolatedDatabase`, que crea una base de datos en ese
contenedor y — este es el punto del slice — ejecuta
`ego_persistence::postgres::migrations::run()` contra ella directamente. **Sin plantilla, sin
clon**: la corrida de migraciones *es* la invariante, así que no puede pre-aplicarse una vez
y heredarse.

Exactamente un archivo de test lo usa. Exactamente cuatro afirmaciones corren ahí:

| # | Afirmación | Por qué es sensible a la versión |
|---|---|---|
| T0 | `current_setting('server_version_num')::int` está en `[140000, 150000)` | Anti-vacuidad. Sin ella, una errata en el tag corre el "slice de PG14" sobre PG16 e IS-9 no prueba nada — el mismo fallo de tres-conjuntos-vacíos para el que el guard del ledger tiene su propio control |
| T1 | El conjunto completo de migraciones 001–012 aplica limpiamente | 008, 011 y 012 existen *porque* `NULLS NOT DISTINCT` es de PostgreSQL 15+ y el piso es 14; el propio comentario de 008 registra que es un error de sintaxis en la imagen 14. 012 además usa `DELETE … USING (SELECT … ROW_NUMBER() OVER …)` |
| T2 | Un `(aggregate_type, aggregate_id, version)` duplicado bajo `tenant_id IS NULL` se rechaza con `23505` | La consecuencia conductual de esa decisión. La afirmación de catálogo fija la forma del índice; solo un insert prueba que PG14 lo hace cumplir |
| T3 | Migración 007 + camino de commit de `backfill_aggregate_type` + ida y vuelta de `revert_aggregate_type_column` | `ALTER TABLE … SET NOT NULL` dentro de una transacción, luego `DROP COLUMN`, sobre la versión piso |

**Explícitamente NO sobre PG14:** IS-1, IS-2, IS-3, IS-6, los casos C1/C2 de abort y rollback
de IS-4, y los dieciséis tests preexistentes. Esos ejercitan comportamiento de transacción,
lock y pool que no diverge entre 14 y 16; correrlos dos veces es exactamente la acreción que
nombra **R-9**.

**Alternativas consideradas.** (a) Un target de test `pg14` separado con su propio paso de
runner — rechazado: un segundo ciclo de vida para un archivo, y el guard del ledger
necesitaría una segunda fuente. (b) Condicionar el slice a una variable de entorno —
rechazado por el propio argumento de D-10: una verificación opt-in del piso declarado es
deuda de verificación con una bandera puesta. (c) Que el test arranque su propio contenedor —
rechazado: es lo único que las convenciones de esta suite prohíben de plano.

**Costo.** Un arranque extra de contenedor (~1.8s medido para el de PG16) más una corrida de
migraciones (~0.5s). Reportado en la línea de tiempos existente, que gana los instantes de
aprovisionamiento y reclamación de PG14.

### AD-7 — IS-8: el procedimiento de mutación, como receta repetible

Por mutación, exactamente estos pasos, registrados como una fila en la tabla de mutaciones ya
existente de `integration-tests/README.md`:

```
1. shasum -a 256 <archivo>                                  registrar ANTES
2. aplicar la edición exacta nombrada
3. cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite
4. registrar: qué tests fallaron, por nombre, con el mensaje de la afirmación
5. git checkout -- <archivo>
6. shasum -a 256 <archivo>                                  DEBE ser igual al paso 1
7. repetir el paso 3                                        control negativo: todo verde
```

| ID | Objetivo | Edición exacta | Esperado |
|---|---|---|---|
| M1a | `UPDATE` de takeover en `reservation.rs` | `.bind(now)` → `.bind(now - chrono::Duration::days(3650))`, neutralizando `lease_until <= $7` | `fencing_window_postgres` **falla**; `lease_contention_postgres` sigue verde |
| M1b | el mismo `UPDATE` | `AND fencing_token = $6` → `AND $6 = $6` | suite entera verde |
| M1c | el mismo `UPDATE` | las dos anteriores juntas | `lease_contention_postgres` **falla** con seis `TakenOver` donde se exige uno; `fencing_window_postgres` también falla |
| M2 | `aggregate_type_backfill.rs`, rama `StreamVersionsAreNotConsecutiveFromOne` | `tx.rollback().await?` → `tx.commit().await?` | C2 de `aggregate_type_backfill_postgres` **falla** en el digest; todo lo demás verde |
| M3 | `event_store.rs`, `PostgresEventStoreUnitOfWork::append` | `.execute(&mut *self.tx)` → `.execute(&self.pool)` | `durable_store_conformance_postgres` **falla** en "un append no commiteado no debe ser visible a un lector"; los llamadores de conformidad in-memory siguen verdes |

**M1a/M1b/M1c son tres filas, no una, y ese es el hallazgo.** El "exactamente un ganador" de
IS-3 descansa sobre la **conjunción** de dos predicados que aquí son individualmente
suficientes cada uno: neutraliza el chequeo de lease y el compare-and-swap igual admite un
ganador; neutraliza el CAS y el chequeo de lease igual lo hace. Solo neutralizando ambos se
rompe. `reservation.rs:300-309` ya registra que el CAS es redundante dado el predicado de
lease y dice que la suite entera queda verde sin él — M1b re-mide esa afirmación en vez de
heredarla.

**Esto refuta SC-8 tal como está escrito.** SC-8 exige que con el mecanismo neutralizado "el
test nuevo falle **y la suite preexistente siga verde**". Para M1c esa segunda cláusula es
falsa: la única mutación que rompe IS-3 también rompe `fencing_window_postgres`, porque ambos
tests cargan el mismo predicado. El enunciado demostrable es *"el test nuevo falla, y ningún
test que no nombre ese predicado falla"* — radio de impacto, no verdor global. Marcado para
revisión cruzada con spec; M2 y M3 satisfacen SC-8 exactamente como está escrito.

## Flujo de Datos

```
run-suite ──┬─→ guard del ledger (hermético, sin contenedor)   falla en ms ante deriva
            ├─→ contenedor PG16 ─→ ego_template (migrada una vez)
            │                        │
            │                        └─→ ego_test_{n} (clon, por test)
            │                              ├─ IS-1/IS-6  arneses de conformidad (defs compartidas)
            │                              ├─ IS-2/IS-5  LOCK TABLE + wait_until_blocked(4)
            │                              ├─ IS-3       FOR UPDATE  + wait_until_blocked(6)
            │                              └─ IS-4       backfill, digest antes/después
            ├─→ contenedor PG14 ─→ pg14_test_{n} (las migraciones corren AQUÍ, no se clonan)
            │                              └─ IS-9       guard T0, T1, T2, T3
            └─→ rm() de ambos, dentro de un runtime vivo, en todo camino
```

## Cambios de Archivos

| Archivo | Acción | Descripción |
|---------|--------|-------------|
| `integration-tests/tests/infrastructure/durable_store_conformance_postgres.rs` | Crear | IS-1 + IS-6 |
| `integration-tests/tests/infrastructure/events_identity_race_postgres.rs` | Crear | IS-2 + IS-5 |
| `integration-tests/tests/infrastructure/lease_contention_postgres.rs` | Crear | IS-3 |
| `integration-tests/tests/infrastructure/aggregate_type_backfill_postgres.rs` | Crear | IS-4, casos C1–C4 |
| `integration-tests/tests/infrastructure/pg14_compatibility.rs` | Crear | IS-9, T0–T3 |
| `integration-tests/tests/infrastructure.rs` | Modificar | Cinco registros `mod`; el guard del ledger falla sin ellos |
| `integration-tests/README.md` | Modificar | Cinco filas de ledger, dos categorías nuevas, conteos actualizados, cinco filas de mutación |
| `integration-tests/src/lib.rs` | Modificar | `wait_until_blocked`, `pg14_database` |
| `integration-tests/src/main.rs` | Modificar | Segundo contenedor: arranque, publicación, reclamación en todo camino, reporte de tiempos |
| `integration-tests/tests/infrastructure/fencing_window_postgres.rs` | Modificar | Su sondeo privado pasa a ser una llamada a `wait_until_blocked`; comentario de documentación y todas sus afirmaciones sin cambios |
| `crates/testkit/src/{event_store.rs,reservation_conformance.rs}` | Sin cambios | Reutilizados textualmente (D-5) |
| `crates/persistence/**`, `migrations/**` | Sin cambios | Ejercitados, no modificados (D-8) |
| `Cargo.toml` raíz, `cargo test --workspace` | Intactos | La raíz sigue libre de Docker |

Dos de estas filas **no** están en la tabla de Áreas Afectadas de la propuesta: `src/lib.rs` y
`src/main.rs`. IS-9 y el sondeo compartido no pueden entregarse sin ellas. Marcado para
revisión cruzada con spec; ambas están dentro de `integration-tests/`, así que el plan de
rollback no se ve afectado.

## Interfaces / Contratos

```rust
// integration-tests/src/lib.rs — nuevo, las dos únicas adiciones
pub async fn wait_until_blocked(observer: &PgPool, statement_like: &str, expected: usize);
pub async fn pg14_database() -> IsolatedDatabase;   // migrada en el lugar, no clonada
```

Sin cambios de interfaz de producción. Sin cambios en `ego-testkit` (D-5 se sostiene).

## Estrategia de Pruebas

| Capa | Qué se prueba | Enfoque |
|------|---------------|---------|
| Unitaria | Nada nuevo | Toda propiedad in-process en alcance ya está cubierta; D-3 eliminó el único límite que parecía abierto |
| Integración (PG16) | IS-1..IS-6 | Cinco funciones de test en cuatro archivos, sobre el contenedor compartido y la base por test existentes |
| Integración (PG14) | IS-9 | Un archivo, un contenedor, cuatro afirmaciones, migraciones aplicadas en el lugar |
| Ledger | Coincidencia entre registro/fila/archivo | `tests/ledger.rs`, hermético, corre antes de aprovisionar |
| Adversarial | IS-8 | Cinco mutaciones registradas, el procedimiento de AD-7, prueba de restauración por SHA-256 |

Pronóstico de presupuesto: IS-1 ≈ 21 reseteos de reservaciones más ~40 idas y vueltas del
event store en localhost; IS-2/IS-3 están dominados por sus plazos de sondeo, que no se
alcanzan en una corrida que pasa; IS-9 agrega un arranque de contenedor más una corrida de
migraciones. Reloj de pared adicional estimado ~4–6s sobre una base de ~16s, bien dentro del
presupuesto de ≤5 min. Compilación y ejecución siguen reportándose por separado, como el
runner ya hace.

## Matriz de Amenazas

N/A — no se agrega ningún límite de enrutamiento, shell, subproceso, automatización de
VCS/PR, clasificación de archivos ejecutables ni integración de procesos. Los
`Command::new(env!("CARGO"))` existentes del runner quedan sin cambios, y ningún argumento
nuevo se deriva de datos de test. El único límite de recursos nuevo es el segundo contenedor,
cubierto por un requisito explícito de diseño: **ambos contenedores se reclaman en todo camino
de salida, esperados dentro de un runtime vivo, antes de devolver el código de salida de la
suite** — exactamente el fallo que el runner fue escrito para cerrar, y una regresión aquí se
filtraría en silencio tras una suite verde.

## Migración / Despliegue

Sin migración. Solo tests y aditivo. Revertir es borrar los cinco archivos, sus cinco
registros de módulo y sus cinco filas de ledger, y revertir los cuatro archivos modificados;
el guard del ledger verifica que las tres fuentes coincidan en ambas direcciones, así que un
revert parcial falla ruidosamente. La entrega se corta naturalmente para el presupuesto de
revisión de 400 líneas: IS-1 por sí solo cierra el AC10 de #275 y es un primer PR completo.

## Preguntas Abiertas

- [ ] **Q1 — Alcance de IS-2.** La primera cláusula de SC-2 ya la cumple IS-1 (AD-3).
      Confirmar con `spec.md` que el texto de requisito de IS-2 es la rama de relectura
      post-`23505`, no el pre-chequeo, antes del primer RED.
- [ ] **Q2 — Completitud de IS-4.** C2 (RolledBack) es necesario para que IS-4 trate de
      transacciones siquiera (AD-5), y la redacción de la propuesta no lo nombra. Confirmar
      que el spec lo admite.
- [ ] **Q3 — Redacción de SC-8.** El verdor global bajo M1c no es alcanzable (AD-7).
      Confirmar que el spec enuncie radio de impacto en vez de verdor global.
- [ ] **Q4 — Áreas Afectadas.** `integration-tests/src/{lib,main}.rs` necesitan filas en la
      tabla de la propuesta.
