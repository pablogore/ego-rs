# Diseño: STOOLAP-RS-01 — Stores durables de read-side sobre Stoolap

> Compañero en español. Canónico / inglés: `design.md` (encabezados e identificadores AD 1:1).

## Enfoque técnico

Tres stores respaldados por Stoolap detrás de una nueva feature `read-side` de Cargo en el crate
existente `crates/persistence-stoolap`, cada uno implementando un puerto ya entregado de
`crates/persistence-api/src/read_side/`, sin cambio de contrato:

- `StoolapOffsetStore` — `read_offset`/`write_offset`, last-write-wins, con clave
  `(projection_id, tag, tenant)`.
- `StoolapDedupStore` — `seen`/`mark_seen`, con clave `(projection_id, tag, event_id)`, sin tenant.
- `StoolapReadSideClaimStore` — `try_claim`/`renew`/`release`, con clave
  `ClaimId { projection_id, tag, tenant }`, protegido por `FencingToken`.

La implementación de referencia para el *algoritmo* es `StoolapOperationReservationStore`
(`crates/persistence-stoolap/src/operation/reservation.rs`), no los stores de read-side de
Postgres: solo se usan formas ya probadas contra Stoolap 0.4 en este repositorio. La referencia
para la *semántica del puerto* es `crates/persistence/src/postgres/read_side_{offset,dedup,claim}.rs`.

Spec: `persistence-stoolap-read-side` (capacidad nueva). No se modifica ninguna spec existente —
los gates de `Profile::Production` (`builder.rs:906-946`) ya existen y se satisfacen, no se cambian.

## Decisiones de arquitectura

### AD-1: Una feature `read-side` nueva en el crate existente, no un crate nuevo

**Elección**: `read-side = ["dep:tokio", "dep:async-trait", "dep:chrono", "dep:ego-domain"]` en
`crates/persistence-stoolap/Cargo.toml`, junto a las features existentes `event-sourcing` (`:52`) y
`operation-reservation` (`:56-62`).

**Alternativas consideradas**: un crate nuevo `ego-persistence-stoolap-read-side`; reutilizar
`operation-reservation`.

**Justificación**: Las cuatro dependencias opcionales **ya están declaradas** en ese manifiesto
(`chrono` `:16`, `tokio` `:17`, `async-trait` `:18`, `ego-domain` `:21`) — cero agregados al
manifiesto, cero aristas transitivas nuevas, exactamente como afirma la propuesta. `base64` se
excluye deliberadamente: ninguna columna de read-side almacena bytes. Reutilizar
`operation-reservation` entregaría a todo consumidor de read-side la dependencia `base64` y el
store de reservas, anulando la razón por la que AD-7 de STOOLAP-S3 creó una feature separada. Un
crate nuevo duplicaría `stoolap_common`, `dsn_for` y el chequeo fail-closed de `sync=full` para
tres stores pequeños.

`crates/service-sdk/Cargo.toml:75-77` ya depende de este crate como dev-dependency con
`features = ["operation-reservation"]`; esa lista suma `"read-side"` (AD-13).

### AD-2: `src/read_side/{mod,offset,dedup,claim}.rs`; tres `open()` independientes, sin tipo agrupador

**Elección**: `src/read_side/mod.rs` declarando `pub mod offset; pub mod dedup; pub mod claim;`,
con la puerta `#[cfg(feature = "read-side")] pub mod read_side;` en `lib.rs` y tres `pub use` en la
raíz del crate — la forma idéntica que ya tiene `operation/` (`lib.rs:13-14,22-23`). Cada store
tiene su propio `open(path: &Path, …) -> Result<Self, PortError>`. **Sin** factory agrupador
`StoolapReadSideStores`.

**Alternativas consideradas**: un solo archivo `read_side.rs`; un factory
`StoolapReadSideStores::open(path)` que devuelva el trío, espejando
`reference_app::read_side::ReadSideProgressStores::postgres`.

**Justificación**: Un archivo por puerto coincide con cómo archivan esta capacidad tanto el crate
de puertos (`persistence-api/src/read_side/{offset,dedup,claim}.rs`) como el adaptador Postgres
(`persistence/src/postgres/read_side_{offset,dedup,claim}.rs`). El factory sería un tipo con un
solo llamador (el test de composición) que además el gate no quiere: consume tres `Arc` separados.
Tres llamadas a `open()` sobre una misma ruta es además evidencia *más fuerte*: ejercita la
propiedad de motor compartido (STOOLAP-S3 design.md "Alcance de concurrencia") en vez de
ocultarla. Disparador para reconsiderarlo: que un host fuera de este repositorio lo pida.

### AD-3: Esquema mínimo y nativo de Stoolap — `UNIQUE`, sin `PRIMARY KEY`, sin `CHECK`, sin timestamps más allá del lease

**Elección**: tres tablas creadas por el `open()` de su propio store, nombradas como las tablas de
Postgres para que el esquema lógico se lea igual en ambos backends.

```sql
CREATE TABLE IF NOT EXISTS projection_offsets (
    projection_id TEXT    NOT NULL,
    tag           TEXT    NOT NULL,
    tenant        TEXT    NOT NULL,
    offset_value  INTEGER NOT NULL,        -- i64; Offset tiene una sola variante, Sequence(i64)
    UNIQUE (projection_id, tag, tenant)
);

CREATE TABLE IF NOT EXISTS projection_dedup (
    projection_id TEXT NOT NULL,
    tag           TEXT NOT NULL,
    event_id      TEXT NOT NULL,           -- sin columna tenant: el puerto no recibe tenant
    UNIQUE (projection_id, tag, event_id)
);

CREATE TABLE IF NOT EXISTS projection_claims (
    projection_id TEXT      NOT NULL,
    tag           TEXT      NOT NULL,
    tenant        TEXT      NOT NULL,
    owner_id      TEXT      NOT NULL,
    fencing_token INTEGER   NOT NULL,      -- i64, siempre >= 1
    lease_until   TIMESTAMP NOT NULL,
    UNIQUE (projection_id, tag, tenant)
);
```

**Justificación, punto por punto**:

| Elección | Por qué |
|---|---|
| `UNIQUE`, nunca `PRIMARY KEY` | Stoolap 0.4 **parsea pero no aplica** una `PRIMARY KEY` compuesta a nivel de tabla (sin constraint, sin índice); `UNIQUE` sí se aplica y es contra lo que hace match `ON CONFLICT`. Confirmado y repetido en todos los stores Stoolap del árbol: `reservation.rs:64,165-168`, `event_store.rs:50,63`, `repository.rs:16`, `snapshot.rs:31`, `effect-store/src/stoolap/mod.rs:230-236` |
| Sin `CHECK (fencing_token > 0)` | Ninguna tabla Stoolap de este codebase usa `CHECK`. El invariante lo impone Rust vía el propio tipo `FencingToken` más el rechazo de `raw <= 0` en `token_from_storage` (AD-11) — una sola definición, no dos |
| Sin triggers ni stored procedures | No existen en ningún lado de este codebase, en ninguno de los dos backends |
| Dedup sin columna `seen_at` | Poda/TTL/retención es un Non-Goal explícito de la spec; una columna que nadie lee es peso muerto e invitaría a la feature de retención que la spec prohíbe |
| `tenant` se guarda tal cual; **no** se usa `stoolap_common::encode_tenant` | Los puertos de read-side reciben `tenant: &str`, no `Option<TenantId>` — no hay caso de tenant ausente que codificar. Postgres también lo enlaza directo (`read_side_offset.rs:79`). Usar el centinela `""` aquí inventaría una distinción que el puerto no tiene |
| `lease_until TIMESTAMP` | Mismo tipo de columna y misma ruta de lectura que `reservation.rs:60`; en Stoolap 0.4 no existe `FromValue for DateTime<Utc>`, así que se lee vía `row.get_value(idx)` con match contra `Value::Timestamp` (`reservation.rs:117-128`) |

### AD-4: Las escrituras de offset y dedup usan solo formas de sentencia ya probadas contra Stoolap 0.4

**Elección**:

- `mark_seen`: una sentencia, `INSERT INTO projection_dedup (…) VALUES ($1,$2,$3) ON CONFLICT
  (projection_id, tag, event_id) DO NOTHING`. Idempotente por construcción; una repetición afecta
  cero filas y **no** es un error. `seen`: `SELECT 1 … LIMIT 1`, la presencia es la respuesta.
- `write_offset`: **UPDATE primero**, luego insertar-si-ausente, luego re-UPDATE solo si un
  competidor insertó:

```
1. UPDATE projection_offsets SET offset_value=$v WHERE projection_id=$p AND tag=$t AND tenant=$tn
   affected >= 1  -> listo                                 (estado estable: UNA sentencia)
2. INSERT INTO projection_offsets (…) VALUES (…)
     ON CONFLICT (projection_id, tag, tenant) DO NOTHING
   inserted == 1  -> listo                                 (primera escritura para esa clave)
3. si no, un escritor concurrente insertó en la ventana -> repetir la sentencia 1, listo
```

**Alternativas consideradas**: `INSERT … ON CONFLICT … DO UPDATE SET offset_value = EXCLUDED.…`,
que es lo que usa Postgres (`read_side_offset.rs:94-99`).

**Justificación**: `ON CONFLICT … DO UPDATE` no se usa **en ningún lado** contra Stoolap en este
repositorio — todo upsert Stoolap del árbol es o bien `DO NOTHING` (`reservation.rs:256`,
`effect-store/src/stoolap/mod.rs:530,775`) o bien un `SELECT`-luego-`INSERT`-o-`UPDATE` explícito
(`snapshot.rs`). Su comportamiento en Stoolap 0.4 no está probado, y la propia fila de riesgo de la
propuesta indica portar la forma probada, no la de Postgres. UPDATE-primero se elige sobre
INSERT-luego-UPDATE porque las escrituras de offset son la ruta caliente por lote y la fila existe
para toda escritura después de la primera, así que el estado estable cuesta una sentencia, no dos.
La corrección bajo el contrato **last-write-wins** del puerto no se ve afectada por cuál competidor
sobrevive — el trait no expresa compare-and-swap y la spec prohíbe agregarlo.

### AD-5: `try_claim` porta el CAS de dos sentencias del store de reservas

**Elección** (`now = clock.now()`, `lease_until` es el parámetro del llamador):

```
1. INSERT INTO projection_claims (projection_id, tag, tenant, owner_id, fencing_token, lease_until)
   VALUES ($p, $t, $tn, $owner, 1, $lease_until)
   ON CONFLICT (projection_id, tag, tenant) DO NOTHING
   inserted == 1  ->  Ok(Some(fence{ FencingToken::initial() }))          # concesión fresca

2. SELECT owner_id, fencing_token, lease_until FROM projection_claims WHERE $p AND $t AND $tn
   sin fila       ->  Err(Transient("la fila del claim desapareció tras el conflicto; reintentar"))

3. now < row.lease_until  ->  Ok(None)                                    # lease vivo: rechazo

4. displaced = token_from_storage(row.fencing_token)?
   next      = displaced.next().ok_or(ClaimError::FencingExhausted)?
   UPDATE projection_claims
      SET owner_id=$owner, fencing_token=$next, lease_until=$lease_until
    WHERE projection_id=$p AND tag=$t AND tenant=$tn
      AND fencing_token = $displaced      -- CAS contra el token que observó el paso 2
      AND lease_until  <= $now            -- reverificado contra la fila VIVA, no contra la lectura
   affected == 1  ->  Ok(Some(fence{ next }))                             # takeover

5. affected == 0  ->  Ok(None)            # un par tomó el control o el dueño renovó en la ventana
```

**Alternativas consideradas**: el
`INSERT … ON CONFLICT … DO UPDATE … WHERE projection_claims.lease_until <= $now RETURNING
fencing_token` de una sola sentencia de Postgres (`read_side_claim.rs:176-195`).

**Justificación**: La forma de Postgres depende de tres cosas no probadas contra Stoolap 0.4 —
`DO UPDATE`, una cláusula `WHERE` adherida a `DO UPDATE`, y `RETURNING`. La forma de dos sentencias
ya está probada bajo estrés para concurrencia intraproceso en este crate
(`tests/reservation_conformance.rs:192-245`).

**Por qué no es posible una actualización perdida.** El `WHERE` de la sentencia 4 reafirma *ambos*
valores que leyó el paso 2. Dos tasks que observan concurrentemente `fencing_token = 5,
lease_until <= now` calculan ambos `next = 6`, pero como máximo un `UPDATE` puede confirmarse
contra esa versión de fila bajo el MVCC de Stoolap. El perdedor se resuelve de exactamente dos
maneras, nunca de una tercera:

| Lo que observa el perdedor | Clasificación | Por qué es correcto |
|---|---|---|
| `affected == 0` (el token confirmado ya es 6, o el dueño renovó y `lease_until > now`) | `Ok(None)` | Un lease vivo tiene el claim. Es un rechazo, no un fallo — exactamente el contrato del puerto |
| un error crudo para el que `stoolap_common::is_write_conflict` es `true` (`UniqueConstraint`, `TransactionAborted`, `LockAcquisitionFailed`, `DatabaseLocked`, el mensaje fijado `"uncommitted changes from transaction"`) | `ClaimError::Transient(msg)` | La sentencia no llegó a un veredicto; reintentar es seguro y no dice nada sobre la propiedad |

Las carreras de la sentencia 1 se resuelven igual: sobre una tabla vacía exactamente un `INSERT`
tiene éxito (`UNIQUE` más `DO NOTHING` significa que el perdedor ve `0`, no una violación), el
perdedor cae al paso 2, lee la fila fresca del ganador y rechaza en el paso 3. Exactamente un
`Ok(Some(fence))` por lease vivo, siempre.

A diferencia de `reserve`, el paso 5 **no** necesita relectura: el puerto de claim no tiene la
distinción `OwnedInProgress` / `OtherInProgress` que recuperar — `Ok(None)` es toda la respuesta.

### AD-6: `renew` y `release` son una sentencia y un helper concreto; **sin** combinador compartido `mutate_owned`

**Elección**: un `fn set_lease(&self, fence, new_lease_until) -> Result<(), ClaimError>` privado
dentro de `read_side/claim.rs`, que emite la sentencia de abajo; `renew` lo llama con el
`lease_until` del llamador, `release` lo llama con `now`. `affected == 0` ⇒ `ClaimError::StaleOwner`.

```sql
UPDATE projection_claims SET lease_until = $1
 WHERE projection_id = $2 AND tag = $3 AND tenant = $4
   AND owner_id = $5 AND fencing_token = $6
   AND lease_until > $7            -- $7 = now; un dueño vencido no puede resucitar su claim
```

**Alternativas consideradas**: (i) escribir la sentencia dos veces en línea, como hace
`reservation.rs:382-478` con sus tres mutadores; (ii) promover un combinador genérico
`mutate_owned<F>` entre stores ahora que este crate tendría cinco mutadores con verificación de
fence, que es justo el disparador que nombra el comentario ponytail de `reservation.rs:382-387`
("promote … if a fourth mutator with the same ownership predicate shows up").

**Justificación — el umbral del comentario se responde explícitamente, no se deja abierto.** Un
combinador *entre stores* se rechaza: los dos módulos difieren en tipo de error (`ReservationError`
vs `ClaimError`), aridad de clave (2 columnas vs 3) y predicado extra (`state='in_progress'` vs
ninguno), así que un genérico compartido necesitaría un mapeador de error, una tupla de clave y un
fragmento de predicado como parámetros — más código en cada uno de los cinco sitios de llamada que
las cinco sentencias que reemplaza. Los tres mutadores de `reservation.rs` quedan por lo tanto en
línea, intactos por este cambio. Dentro de `claim.rs` el caso es distinto y más fuerte que
"parecido": `renew` y `release` son la **sentencia idéntica** con un solo valor enlazado distinto —
el propio `read_side_claim.rs:210-254` de Postgres enlaza `lease_until` en uno y `now` en el otro
contra una consulta byte a byte igual. Un helper concreto de 15 líneas, no un combinador genérico.

**Release es una expiración, nunca un `DELETE`**: poner `lease_until = now` conserva la fila, así
que el fencing token queda estrictamente monótono al cruzar el release y el claim es reclamable de
inmediato — el requisito que declara la spec y la regla que el doc de módulo de Postgres
(`read_side_claim.rs:20-25`) ya lleva.

### AD-7: El store de claim posee un `Clock` inyectado; el llamador posee `lease_until`

**Elección**: `StoolapReadSideClaimStore::open(path: &Path, clock: Arc<dyn ego_domain::Clock>)`.
Toda comparación de expiración lee `clock.now()`; el store nunca llama a `Utc::now()`,
`SystemTime::now()` ni a un `now()` de SQL. `lease_until` siempre llega como parámetro
`DateTime<Utc>` en `try_claim`/`renew` y se almacena tal cual. `StoolapOffsetStore` y
`StoolapDedupStore` no reciben clock alguno — ninguno de esos puertos tiene dimensión temporal.

**Justificación**: Es la forma que ya tienen ambas implementaciones existentes de esta misma
familia de puertos — `PostgreSQLReadSideClaimStore { pool, clock }`
(`read_side_claim.rs:83-86,105-107,163`) y `StoolapOperationReservationStore { db, clock }`
(`reservation.rs:132-135,153,308`) — y es la única forma que permite la firma del puerto:
`try_claim(claim_id, owner_id, lease_until)` no lleva parámetro `now`, así que "¿el lease del
titular sigue vivo?" es irrespondible sin un clock que el store posea. El determinismo se preserva
porque el clock lo inyecta la raíz de composición: los tests manejan `ego_testkit::TestClock` y lo
avanzan explícitamente, tal como hace `reservation_conformance.rs:114`.

**Reconciliación con el texto de la spec, declarada en vez de disimulada.** El escenario "Lease
Expiry Is Always Caller-Computed" de la spec dice "never on a clock read performed inside the
store". Tomado literalmente eso es inimplementable contra el trait sin modificar, y ninguna
implementación del árbol de ningún puerto con lease lo satisface. La lectura satisfacible — y la
que este diseño implementa — es: el *límite del lease* siempre lo calcula el llamador y nunca el
store, y el "now" propio del store viene de un `Clock` inyectado y provisto por el llamador, nunca
del tiempo ambiental del sistema. Se marca como riesgo abajo para que `sdd-verify` lea el requisito
igual en vez de reprobarlo.

### AD-8: Los errores se clasifican `Transient` vs `Fatal` a través de `is_write_conflict`

**Elección**: para los tres stores, un `stoolap::Error` crudo para el que
`stoolap_common::is_write_conflict` devuelva `true` se mapea a la variante `Transient` del puerto;
todo lo demás se mapea a `Fatal`. `affected == 0` en una mutación con verificación de fence se
mapea a `StaleOwner` (AD-6), nunca a `Transient`.

**Justificación**: `OffsetStoreError`, `DedupStoreError` y `ClaimError` tienen cada uno variantes
`Transient` y `Fatal`, así que estos tres stores pueden clasificar con honestidad — a diferencia de
`OperationReservationStore`, cuyo puerto no tiene ninguna de las dos, forzando a `reservation.rs` a
colar la pista de reintento dentro de `Backend("…retry")` (STOOLAP-S3 AD-5). El default es fallar
fuerte: el brazo comodín de `is_write_conflict` mantiene fuera de `Transient` todo lo no
reconocido. Esto es lo que hace literalmente satisfacible la cláusula de concurrencia de la spec:
"`Ok(None)` **o** un `Transient` seguro de reintentar".

### AD-9: `open()` falla cerrado ante `sync=full`; `is_durable()` se rededuce del DSN vivo

**Elección**: el `open()` de cada store construye su DSN con `stoolap_common::dsn_for(path)`, abre,
y rechaza con la variante `Fatal` del puerto si `dsn_declares_sync_full(db.dsn())` es falso, antes
de emitir el `CREATE TABLE`. `is_durable()` devuelve `dsn_declares_sync_full(self.db.dsn())` — no
un `true` hardcodeado.

**Justificación**: Es literalmente el patrón que ya comparten `reservation.rs:153-173,235-237`,
`snapshot.rs` y `event_store.rs` (STOOLAP-S3 AD-9). Un `true` hardcodeado dejaría que
`is_durable()` mienta sobre cómo se abrió el store, que es exactamente lo que lee el gate de
`Profile::Production`.

### AD-10: `run_blocking` queda duplicado por store; **no** se centraliza en `stoolap_common`

**Elección**: cada uno de los tres stores nuevos lleva su propio
`async fn run_blocking<F, R>(&self, f: F) -> Result<R, PortError>` privado que entrega la clausura
a `tokio::task::spawn_blocking` sobre un `Database` clonado — nunca `block_in_place`.

**Alternativas consideradas**: promover un único helper genérico a `stoolap_common` ahora que seis
stores lo usarían (`effect-store/src/stoolap/mod.rs:277`, `event_store.rs:236`,
`reservation.rs:177`, más estos tres).

**Justificación**: La parte genuinamente compartida son cuatro líneas
(`spawn_blocking(move || f(&db)).await`); el resto es el mapeo por puerto de un `JoinError` a la
variante de fallo propia de ese puerto, y hay seis distintas. Una versión centralizada necesita un
parámetro `on_panic: impl FnOnce(String) -> E` enhebrado por unos treinta sitios de llamada — más
código agregado en los sitios de llamada que el removido de las definiciones, y además editaría
tres stores ya entregados y por lo demás intactos, gastando presupuesto de revisión que este cambio
usa mejor en otra parte. `stoolap_common` existe para lo que es *idéntico* entre stores (`dsn_for`,
`encode_tenant`, `is_write_conflict`, `dsn_declares_sync_full`), y esto no lo es. `block_in_place`
sigue prohibido: entra en pánico fuera de un runtime multihilo y rompería los `#[tokio::test]` de
hilo actual (`reservation.rs:14-19`).

### AD-11: `token_for_storage` / `token_from_storage` se mueven a `stoolap_common` como `pub(crate)`

**Elección**: levantar las dos guardas i64↔`FencingToken` de `operation/reservation.rs:76-93` hacia
`persistence/stoolap_common.rs` como `pub(crate)`, y que tanto `operation/reservation.rs` como
`read_side/claim.rs` las importen. `read_side/claim.rs` agrega un shim
`to_claim_error(ReservationError) -> ClaimError`, espejando
`crates/persistence/src/postgres/read_side_claim.rs:51-57` textualmente.

**Alternativas consideradas**: (i) subir las dos funciones a `pub(crate)` **en su lugar**, el
espejo literal de lo que hizo Postgres (`postgres/reservation.rs:107,124` son `pub(crate)`
precisamente para que `read_side_claim.rs:40` las reutilice); (ii) duplicarlas por tercera vez
dentro de `read_side/`, que es lo que hizo STOOLAP-S3 AD-10 cuando la frontera era entre crates.

**Justificación**: La reutilización de Postgres funciona porque ambos módulos se compilan
incondicionalmente. Aquí no: `operation/reservation.rs` está detrás de
`#[cfg(feature = "operation-reservation")]` (`lib.rs:13`), así que una referencia `pub(crate)` en
su lugar desde `read_side` se rompe bajo `--features read-side` a secas, y repararlo haciendo que
`read-side` implique `operation-reservation` entregaría a todo consumidor de read-side la
dependencia `base64` y el store de reservas (contra AD-1). Levantarlas lo resuelve sin acople de
features y sin una tercera copia: ambos tipos que tocan las guardas (`FencingToken`,
`ReservationError`) viven en `ego-persistence-api`
(`persistence-api/src/operation/reservation.rs:300,457`), una dependencia **incondicional**, así
que `stoolap_common` — creado justamente para terminar con este tipo de duplicación por store
(`stoolap_common.rs:1-5`) — puede alojarlas sin arista nueva. Diff: dos funciones movidas, una
línea de import cambiada en un archivo ya entregado.

### AD-12: Solo tests locales de Stoolap — **sin** arneses de conformidad compartidos en `ego-testkit` (resuelve la Pregunta abierta 1)

**Elección**: verificar los tres stores con tests colocados en los módulos más un binario de
integración `crates/persistence-stoolap/tests/read_side_stores.rs`
(`required-features = ["read-side"]`). **No** agregar
`assert_offset_store_conformance` / `assert_dedup_store_conformance` /
`assert_claim_store_conformance` a `crates/testkit/src/lib.rs`.

**Justificación — dimensionada por cantidad de comportamientos, no por simetría**: `OffsetStore` y
`DedupStore` son puertos de dos métodos con unos cinco comportamientos entre ambos (lecturas
ausentes devuelven `None`/`false`, aislamiento por clave, last-write-wins, idempotencia de la
marca, sin poda). Un arnés compartido para cinco aserciones es más andamiaje — genéricos de
clausura factory, un módulo, exports, comentarios de doc — que las aserciones que transporta.
`ReadSideClaimStore` sí se beneficiaría genuinamente de uno (siete comportamientos, dos
implementaciones durables independientes que deben coincidir), **pero un arnés solo vale su costo
cuando un segundo backend pasa por él**, y hacer pasar a `PostgreSQLReadSideClaimStore` implica
tocar Postgres, que los Non-Goals de este cambio prohíben. Un arnés con exactamente un llamador es
la interfaz de una sola implementación que la disciplina de revisión de este repositorio rechaza.
El precedente existente respalda el dimensionamiento en vez de contradecirlo:
`assert_reservation_store_conformance` se justificó por un puerto de siete métodos con tres
implementaciones, y se agregó en el cambio que creó la *segunda*.

**Seguimiento nombrado, no silencio**: `assert_claim_store_conformance` en `ego-testkit`, manejando
tanto `PostgreSQLReadSideClaimStore` como `StoolapReadSideClaimStore`. Su disparador es un tercer
backend de claim o la primera divergencia entre backends — no este cambio.

### AD-13: El test de composición vive en `crates/service-sdk/tests/`, no en `integration-tests/` ni en `persistence-stoolap/tests/` (resuelve la Pregunta abierta 2)

**Elección**: extender `crates/service-sdk/tests/read_side_progress_composition.rs` con la
composición real de `Profile::Production` y su control negativo volátil, sobre un directorio
`tempfile`. La lista de features de la dev-dependency existente en
`crates/service-sdk/Cargo.toml:75-77` suma `"read-side"`.

**Alternativas consideradas**: (i) `integration-tests/`, el precedente de Postgres
(`integration-tests/tests/infrastructure/read_side_progress_postgres.rs:377-451`);
(ii) `crates/persistence-stoolap/tests/`, como hipotetizó la propuesta.

**Justificación**: La opción (ii) no es solo más pesada, **no está disponible**: `ego-service-sdk`
depende de `ego-persistence-stoolap` (`service-sdk/Cargo.toml:75`), así que una dev-dependency en
el sentido contrario cierra un ciclo — y ese manifiesto documenta dos veces que lo rechaza
(`persistence-stoolap/Cargo.toml:28-29,39-41`: *"One direction only … so no cycle"*). Sin
`RuntimeBuilder`/`App`, un test en ese crate solo podría afirmar `is_durable()` sobre un store
aislado, que es exactamente lo que la spec dice que **no** alcanza. La opción (i) carga con todo el
costo del precedente de Postgres sin ninguna de sus razones: `integration-tests/` es un workspace
aparte con una base de datos aprovisionada por contenedor, un binario run-suite y un contrato de
admisión con ledger (AD-14), todo lo cual existe porque PostgreSQL necesita infraestructura
externa. Stoolap es embebido y sobre archivo; un `tempfile::tempdir()` es toda la infraestructura.

La opción (iii), la elegida, no es un punto medio sino el precedente ya establecido en el árbol
para este mismo problema: STOOLAP-S3 lo resolvió idénticamente con
`crates/service-sdk/tests/operation_reservation_gate_composition.rs`, cuyo encabezado declara el
mismo propósito ("cross-backend gate proof … the workspace's REAL implementations … not a synthetic
stub") y cuya tarea 5.3 ya abre un store Stoolap real sobre un `tempdir` y maneja `App::builder()`.
Costo de esta decisión: una palabra en una lista de features de dev-dependency.

### AD-14: Sin delta a `real-infrastructure-verification` (resuelve la Pregunta abierta 3)

**Elección**: este cambio no agrega ningún requisito a
`openspec/specs/real-infrastructure-verification/spec.md`.

**Justificación, leída de esa spec**: su Purpose la acota a *"which invariants MUST be demonstrated
against real PostgreSQL … and the wall-clock budget"*, y sus cinco requisitos son específicos de
PostgreSQL (los dos arneses de conformidad de Postgres, el backfill de la migración 007, el ledger
de admisión de `integration-tests/`, la compatibilidad de versiones PG14/PG16). Sus Non-Goals son
preocupaciones de infraestructura PostgreSQL. Este cambio no aprovisiona infraestructura, no agrega
ningún archivo a `integration-tests/` y no gasta nada del presupuesto de esa suite, así que ningún
requisito de allí queda comprometido. Agregar un requisito Stoolap ensancharía una capacidad de
*metodología* PostgreSQL hacia una general — un segundo lugar donde se define la verificación de
backends durables, la misma trampa que evitó STOOLAP-S3 AD-9. El precedente es decisivo: STOOLAP-S2
y STOOLAP-S3 entregaron ambos durabilidad Stoolap real **y** tests reales de composición de
Producción, y ninguno agregó un delta allí (sus conjuntos de delta son
`persistence-stoolap-event-sourcing` y `{persistence-api-surface, persistence-memory-adapter,
idempotent-command-processing, persistence-stoolap-operation-reservation,
production-composition-hardening}` respectivamente). El requisito de composición de este cambio ya
está declarado donde corresponde: en "A Real Profile::Production Composition Exercises The Gate,
With A Negative Control" de `persistence-stoolap-read-side`.

## Flujo de datos

```
  sesión de read-side ──▶ Arc<dyn OffsetStore | DedupStore | ReadSideClaimStore>
                              │  (parámetros propios construidos aquí, antes del límite)
                              ▼
                        run_blocking ──▶ spawn_blocking ──▶ Database (handle clonado)
                              │                                   │
                              │                un motor global de proceso por DSN
                              │                projection_offsets / _dedup / _claims
                              ▼                     (WAL sync=full, un archivo)
                     Ok / Ok(None) / Transient|Fatal|StaleOwner
                              ▲
                        clock.now()  (solo el store de claim)
```

### Secuencia: dos tasks compiten por `try_claim` sobre un `claim_id` fresco

```
 Task A                Task B              motor Stoolap (una fila, MVCC)
   │ INSERT…DO NOTHING ─────────────────────────▶ fila creada, token=1
   │◀── affected = 1                             │
   │                    │ INSERT…DO NOTHING ────▶ conflicto, no se levanta violación
   │                    │◀── affected = 0        │
   │                    │ SELECT ───────────────▶ owner=A, token=1, lease_until=L
   │                    │◀── fila                │
   │                    │ now < L  ⇒  Ok(None)   │        ← rechazo, no un error
   │◀ Ok(Some(fence{1}))│                        │
```

```
 …más tarde, lease vencido; A y B intentan ambos el takeover desde token=5

 A: SELECT → (5, vencido)       B: SELECT → (5, vencido)
 A: UPDATE … WHERE fencing_token=5 AND lease_until<=now   ⇒ affected=1, token→6
 B: UPDATE … WHERE fencing_token=5 AND lease_until<=now   ⇒ affected=0  ⇒ Ok(None)
                              …o un conflicto de escritura ⇒ ClaimError::Transient (reintentable)
 Nunca: dos Ok(Some) para un mismo lease vivo, y nunca un token que no sea estrictamente mayor.
```

## Cambios de archivos

| Archivo | Acción | Descripción |
|---|---|---|
| `crates/persistence-stoolap/Cargo.toml` | Modificar | Feature `read-side` sobre cuatro deps opcionales ya declaradas; `[[test]] name = "read_side_stores"` con `required-features = ["read-side"]` |
| `crates/persistence-stoolap/src/lib.rs` | Modificar | `#[cfg(feature = "read-side")] pub mod read_side;` + tres `pub use` en la raíz |
| `crates/persistence-stoolap/src/read_side/mod.rs` | Crear | Declaraciones de módulo; la nota de alcance de concurrencia solo intraproceso |
| `crates/persistence-stoolap/src/read_side/offset.rs` | Crear | `StoolapOffsetStore` (AD-4) + tests colocados |
| `crates/persistence-stoolap/src/read_side/dedup.rs` | Crear | `StoolapDedupStore` (AD-4) + tests colocados |
| `crates/persistence-stoolap/src/read_side/claim.rs` | Crear | `StoolapReadSideClaimStore` (AD-5/AD-6), shim `to_claim_error` + tests colocados |
| `crates/persistence-stoolap/src/persistence/stoolap_common.rs` | Modificar | Alojar `token_for_storage`/`token_from_storage` como `pub(crate)` (AD-11) |
| `crates/persistence-stoolap/src/operation/reservation.rs` | Modificar | Dos funciones removidas, una línea de import agregada (AD-11). Sin cambio de comportamiento |
| `crates/persistence-stoolap/tests/read_side_stores.rs` | Crear | Aislamiento, idempotencia, takeover/fencing, carrera de concurrencia, durabilidad por reapertura |
| `crates/service-sdk/Cargo.toml` | Modificar | La lista de features de la dev-dependency suma `"read-side"` (AD-13) |
| `crates/service-sdk/tests/read_side_progress_composition.rs` | Modificar | Composición real de Producción + control negativo volátil (AD-13) |
| `crates/persistence-api/src/read_side/**` | Sin cambios | Contratos consumidos tal cual |
| `crates/persistence/src/postgres/**`, `crates/service-sdk/src/runtime/builder.rs` | Sin cambios | Sin cambio de gate, sin cambio en Postgres |

## Interfaces / Contratos

```rust
// crates/persistence-stoolap/src/read_side/{offset,dedup,claim}.rs
impl StoolapOffsetStore {
    pub async fn open(path: &Path) -> Result<Self, OffsetStoreError>;
}
impl StoolapDedupStore {
    pub async fn open(path: &Path) -> Result<Self, DedupStoreError>;
}
impl StoolapReadSideClaimStore {
    /// `clock` provee el "ahora" solo para comparaciones de expiración; todo
    /// `lease_until` es del llamador (AD-7).
    pub async fn open(path: &Path, clock: Arc<dyn ego_domain::Clock>) -> Result<Self, ClaimError>;
}

// Los tres, idénticamente (AD-9): veraz por construcción, nunca un `true` hardcodeado.
fn is_durable(&self) -> bool { dsn_declares_sync_full(self.db.dsn()) }
```

```toml
# crates/persistence-stoolap/Cargo.toml — toda dep ya declarada (AD-1)
read-side = ["dep:tokio", "dep:async-trait", "dep:chrono", "dep:ego-domain"]

[[test]]
name = "read_side_stores"
required-features = ["read-side"]
```

```rust
// crates/service-sdk/tests/read_side_progress_composition.rs (AD-13), caso positivo
let app = App::builder()
    .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
    .profile(Profile::Production)
    .read_side_progress("users-by-tenant", Arc::new(offset), Arc::new(dedup))
    .read_side_claims(Arc::new(claim))
    .build();                 // Ok — el gate sin modificar acepta tres stores Stoolap durables
// Control negativo: reemplazar exactamente un store por el VolatileOffsetStore que el archivo
// ya tiene ⇒ CompositionError::Validation(RuntimeError::PersistenceNotConfigured(_))
```

## Alcance de concurrencia — qué se afirma y qué no

| Escenario | Soportado | Base |
|---|---|---|
| Un proceso, muchos tasks async, una instancia de store | **Sí** | Toda mutación es una sentencia condicional bajo el MVCC de Stoolap (AD-4/AD-5); el perdedor obtiene `Ok(None)` o un `Transient` reintentable (AD-8) |
| Un proceso, tres instancias de store en una ruta (la composición de producción) | **Sí** | El registro global de proceso de Stoolap comparte un motor vivo por DSN mientras exista un handle; probado para este crate por `tests/reservation_conformance.rs:258-311`. Se vuelve a probar aquí en vez de asumirse |
| Múltiples procesos del SO sobre un archivo | **No soportado, no probado, no afirmado** | Nada en el árbol establece bloqueo entre procesos |
| Multinodo / distribuido / elección de líder | **No soportado, no afirmado** | El mismo precedente de honestidad que el `multi_node_safe: false` de `StoolapEffectStore` |

El doc de módulo de cada store declara este alcance. El fencing hace autoritativo el *resultado del
claim*; no cancela trabajo que un dueño desplazado ya inició — el mismo límite que documentan ambas
implementaciones existentes de puertos con lease.

## Estrategia de pruebas

| Capa | Qué probar | Enfoque |
|---|---|---|
| Unidad — offset | Una clave ausente lee `None`; una escritura en un `(projection_id, tag, tenant)` deja intacta toda otra clave; una escritura repetida sobrescribe sin señal de conflicto; la ruta de fall-through de AD-4 se alcanza en la primera escritura | `#[cfg(test)]` colocado, `tempfile` por test + `stoolap::test_failpoints::FailpointGuard` (la guarda de failpoint global de proceso que toma todo test de BD de este crate, `reservation.rs:589-591`) |
| Unidad — dedup | No visto lee `false`; `mark_seen` y luego `seen` es `true`; un `mark_seen` repetido tiene éxito y sigue `true`; el mismo `event_id` bajo otro `(projection_id, tag)` es independiente | Colocado; misma guarda |
| Unidad — claim | Concesión fresca; rechazo mientras está vivo; takeover tras vencer acuña un token estrictamente mayor; `renew`/`release` rechazan un fence obsoleto y un fence vencido con `StaleOwner` y no mutan nada; tras `release` la **fila sigue existiendo** con lease expirado y es reclamable de inmediato; `FencingToken::next() == None` surge como `FencingExhausted`, nunca como token envuelto | Colocado, `TestClock` avanzado explícitamente (nunca un sleep real) |
| Unidad — los tres | `open()` rechaza una ruta ya tomada por un motor sin `sync=full`; `is_durable()` es `true` tras un `open()` normal | Espeja `reservation.rs:640-652` |
| Integración | **Concurrencia**: varios tasks en un proceso compiten por `try_claim` sobre un `claim_id` fresco; exactamente uno `Ok(Some)`, todo el resto `Ok(None)` o `Transient` — nunca una segunda concesión, nunca `StaleOwner` | `tests/read_side_stores.rs`, `#[tokio::test(flavor = "multi_thread")]` + `tokio::spawn`, la forma que `reservation_conformance.rs:192-245` ya prueba |
| Integración | **Motor compartido**: tres stores abiertos en una ruta observan una sola base de datos | Explícito, según la tabla de concurrencia |
| Integración | **Reinicio**: el estado escrito sobrevive a un cierre limpio y reapertura del mismo archivo, para cada uno de los tres stores | Ver el contrato de reinicio abajo |
| Composición | Una construcción real con `Profile::Production` sobre tres stores Stoolap reales tiene éxito; la misma composición con exactamente un store volátil es rechazada por el gate sin modificar | `crates/service-sdk/tests/read_side_progress_composition.rs` (AD-13) |

**Contrato de reinicio — exactamente qué prueba cada test de reapertura y qué no.**

La forma es idéntica para los tres: `tempfile::tempdir()` → `open()` → escribir estado → **soltar
todos los handles de store para esa ruta** → `open()` la misma ruta otra vez → afirmar. Soltar
*todos* los handles es determinante, no incidental: Stoolap mantiene vivo un motor global de
proceso por DSN mientras exista algún handle, así que un handle sobreviviente haría que el test no
probara nada (`reservation_conformance.rs:129-132` declara la misma advertencia). Cada test usa por
lo tanto un tempdir dedicado con solo su propio store abierto.

| Store | Qué prueba la reapertura | Qué **no** prueba explícitamente |
|---|---|---|
| Offset | `read_offset` para la clave escrita devuelve el `Offset` idéntico, y una clave hermana nunca escrita sigue devolviendo `None` — o sea que el estado reabierto son las filas persistidas, no una tabla vacía reconstruida | Nada sobre caída, `kill -9` ni corte de energía |
| Dedup | `seen()` para la tripleta marcada sigue siendo `true`, y una tripleta no marcada sigue siendo `false` | Ídem |
| Claim | El `try_claim` de un dueño *distinto* sobre el mismo `claim_id` sigue devolviendo `Ok(None)` (el lease no vencido sobrevivió), el fence retenido sigue verificando vía `renew`, y un fence liberado antes del drop reabre como reclamable de inmediato con un token estrictamente mayor | Ídem. Tampoco reapertura entre procesos: el mismo proceso del SO reabre el archivo |

TDD está activo (`openspec/config.yaml`): cada fila aterriza primero en RED. `cargo test
--workspace` no habilita `read-side`, y el `--all-targets` de CI debe seguir compilando sin ella —
de ahí la entrada `required-features` (el precedente de `Cargo.toml:67-74`). Las filas de
`read-side` deben además ejecutarse con `--features read-side`.

## Matriz de amenazas

N/A — no hay frontera de ruteo, shell, subproceso, automatización de VCS/PR, clasificación de
archivos ejecutables ni integración de procesos. La única preocupación adyacente es texto
controlado por el llamador que llega a SQL (`projection_id`, `tag`, `tenant`, `event_id`,
`owner_id` se originan todos fuera del store); se maneja con la misma regla que sigue cada store
aquí: todo valor se enlaza como `$N`, nunca se interpola en la sentencia.

## Migración / Despliegue

Sin migración de datos. Cada `open()` emite `CREATE TABLE IF NOT EXISTS`, así que tanto una base
nueva como una base S1/S2/S3 existente en la misma ruta funcionan. Todo el cambio es aditivo y está
detrás de una feature: con `read-side` apagada, `cargo build` y `cargo test --workspace` se
comportan exactamente como hoy. El movimiento de AD-11 dentro de `stoolap_common` es la única
edición de código ya entregado y preserva el comportamiento.

**División en PRs — la hipótesis de la propuesta, confirmada con una corrección.** La propuesta
supuso PR1 = offset, PR2 = dedup, PR3 = claim, PR4 = composición. El trabajo de diseño confirma los
dos primeros y el último, y divide el tercero:

| PR | Contenido | Líneas est. | Por qué este corte |
|---|---|---|---|
| 1 | Feature de Cargo + `read_side/mod.rs` + cableado en `lib.rs` + `StoolapOffsetStore` + `tests/read_side_stores.rs` (sección offset) | ~280 | Lleva el andamiaje de una sola vez. Autónomo: un store de offset que pasa aislamiento, LWW y reapertura está completo y es reversible por sí solo |
| 2 | `StoolapDedupStore` + sus tests | ~210 | No comparte nada con offset salvo la puerta de feature y la forma de `run_blocking` (~30 líneas de andamiaje, ya aterrizadas en PR1). Fusionar 1+2 daría ~490 — sobre el presupuesto de 400 líneas para ahorrar 30 líneas de duplicación. No conviene |
| 3 | Traslado de AD-11 + `StoolapReadSideClaimStore` + tests unitarios colocados | ~380 | El CAS, el mutador con verificación de fence y las guardas de token son una sola idea revisable |
| 4 | Tests de integración: carrera de concurrencia de claim + durabilidad por reapertura + motor compartido | ~250 | Junto con PR3 esto da ~630. Se separa porque la prueba de concurrencia es el objetivo de revisión de mayor valor del cambio y merece su propio diff, no la cola de uno de 630 líneas |
| 5 | Test de composición + control negativo + la palabra de feature en la dev-dependency de service-sdk | ~120 | Necesita los tres stores; va último por construcción |

`400-line budget risk: Medium` con este corte — se pronostica cada porción bajo 400 con margen. Si
PR3 mide bajo 400 una vez escrito, 3 y 4 pueden fusionarse; `sdd-tasks` debe medir en vez de
asumir. Cadena de rama de feature: PR1 apunta a la rama tracker, cada PR posterior apunta a su
predecesor.

## Preguntas abiertas

Las tres preguntas abiertas de la propuesta quedan resueltas arriba (AD-12, AD-13, AD-14). Quedan
dos incógnitas de nivel de implementación, ambas resolubles por experimento dentro de PR1 y sin
consecuencia contractual:

- [ ] ¿Stoolap 0.4 devuelve `affected` de un `UPDATE … WHERE <sin coincidencia>` simple como `0` y
      no como error? Todo `UPDATE` del árbol lee `affected` así (`reservation.rs:412`,
      `effect-store/src/stoolap/mod.rs:590`), así que la forma está establecida — confirmarlo para
      el paso 1 de AD-4 antes de depender del fall-through, ya que un resultado distinto de cero
      sin coincidencia haría que `write_offset` se saltee su insert.
- [ ] ¿Una columna `INTEGER` hace round-trip exacto de un `offset_value` i64? `fencing_token` ya lo
      hace (`reservation.rs:59,209`), así que es una confirmación, no un riesgo.

## Riesgos que introduce este diseño

| Riesgo | Probabilidad | Mitigación |
|---|---|---|
| `sdd-verify` lee literalmente el "never a clock read performed inside the store" de la spec y reprueba AD-7 | Media | AD-7 declara la reconciliación explícitamente y nombra ambas implementaciones existentes de la misma familia de puertos que leen un clock inyectado. Si verify insiste en la lectura literal, lo que debe cambiar es la frase de la spec, no la implementación, porque la firma del trait no lleva parámetro `now` y está fuera de alcance |
| La clasificación `Transient` del store de claim depende del brazo de texto de mensaje fijado en `is_write_conflict` (`"uncommitted changes from transaction"`) | Baja | Preexistente y ya documentado como frágil (`stoolap_common.rs:58-62`); este cambio agrega un consumidor, no la fragilidad. El test de concurrencia falla ruidosamente si Stoolap cambia el texto |
| Que `ON CONFLICT … DO UPDATE` sí funcione en Stoolap 0.4 después de todo, y la escritura de tres pasos de AD-4 quede sobredimensionada | Baja | Aceptado. El costo son dos sentencias extra solo en la primera escritura; la alternativa es entregar una forma de sentencia no probada en la ruta de durabilidad |
| Que nunca se escriba un `assert_claim_store_conformance` compartido y los dos backends de claim diverjan | Media | Nombrado como seguimiento explícito en AD-12 con su disparador, no dejado implícito |
