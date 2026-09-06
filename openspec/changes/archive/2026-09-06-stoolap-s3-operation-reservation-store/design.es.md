# Diseño: STOOLAP-S3 — Almacén de reservas de operación: señal de durabilidad, implementación Stoolap, compuerta de producción

> Compañero en español. Canónico: `design.md` (mismos encabezados e identificadores de decisión).

## Enfoque técnico

Tres cortes revertibles de forma independiente, en el orden que fija la propuesta.

- **(a)** `OperationReservationStore` incorpora `fn is_durable(&self) -> bool { false }` más una
  implementación de reenvío sobre `Arc<T>`, copiada de `read_side/dedup.rs:33-35,59-67`. Barrido
  de implementadores: el almacén en memoria hereda `false`, el de Postgres sobrescribe a `true`.
- **(b)** `StoolapOperationReservationStore` — un módulo nuevo en `ego-persistence-stoolap`, con
  la forma de `StoolapEffectStore` (`spawn_blocking` sobre la `Database` síncrona, `open()` que
  falla cerrado exigiendo `sync=full`) y con el algoritmo de reserva de
  `PostgresOperationReservationStore`, que es la implementación de referencia de este puerto.
- **(c)** `validate_operation_reservation_profile()` en `RuntimeBuilder`, secuenciada dentro de
  `validate_persistence_profile()` junto a las cuatro compuertas existentes, pasando por el único
  predicado compartido `persistent_entity::profile::require_durably_configured`.

Especificaciones: `persistence-api-surface` (a), `persistence-memory-adapter` (a),
`idempotent-command-processing` (a), `persistence-stoolap-operation-reservation` (b),
`production-composition-hardening` (c).

## Decisiones de arquitectura

### AD-1: La señal de durabilidad es `is_durable()`, y el patrón hermano no es uniforme — la variación se documenta, no se copia a ciegas

**Elección**: `fn is_durable(&self) -> bool { false }` en el trait, más
`impl<T: OperationReservationStore + Send + Sync + ?Sized> OperationReservationStore for Arc<T>`
que reenvía todos los métodos, incluido `is_durable`.

**Alternativas consideradas**: la estructura `capabilities() -> EffectStoreCapabilities` de
`effect-store`; un método obligatorio sin valor por defecto.

**Fundamento**: la estructura `capabilities()` pertenece a otra familia de puertos en otro crate
(`ego-effect-store`); introducirla aquí sería la "segunda forma de expresar durabilidad" que la
propuesta deja fuera de alcance. El valor por defecto `false` es lo que mantiene compilando y
clasificado con honestidad a todo implementador externo.

**Relevamiento verificado de hermanos** (el patrón *no* es idéntico en los cinco — dejarlo asentado):

| Puerto | `is_durable` por defecto | Reenvío `Arc<T>` | Nota |
|---|---|---|---|
| `read_side/dedup.rs:33` | `false` | sí, `:59-67` | patrón completo |
| `read_side/offset.rs:62` | `false` | sí, `:92-99` | patrón completo |
| `read_side/claim.rs:73` | `false` | sí, `:117-124` | patrón completo |
| `persistence/snapshot.rs:19` | `false` | **no** | se sostiene como `Arc<Mutex<dyn Snapshot>>`, no `Arc<dyn _>` |
| `persistence/event_store.rs:54` | `false` | **no** | se sostiene como `Arc<dyn EventStore<E>>`, nunca por valor genérico |

`OperationReservationStore` se sostiene como `Arc<dyn OperationReservationStore>`
(`builder.rs:121`), así que pertenece al primer grupo: se incluye la implementación de reenvío.

### AD-2: La implementación sobre `Arc` es necesaria por paridad y por uso genérico — **no** porque el sitio de llamada de la compuerta fuese a regresionar

**Elección**: incluir la implementación; escribir su prueba contra un genérico
`S: OperationReservationStore` instanciado con `Arc<ConcreteStore>`, **no** contra la compuerta
del builder.

**Fundamento**: esto corrige un supuesto heredado de la propuesta. La compuerta lee
`Option<Arc<dyn OperationReservationStore>>`; sobre `Arc<dyn Trait>`, `is_durable()` resuelve a la
sobrescritura del almacén concreto *en ambos casos*: por el cuerpo de reenvío nuevo, o por
autoderef a `&dyn Trait` sin él. Una prueba que envuelva un almacén durable en `Arc<dyn _>`, lo
registre y afirme que la compuerta acepta **pasaría igual si se borrara la implementación de
reenvío** — es decir, sería vacua. La implementación es realmente determinante donde
`Arc<Concrete>` debe *satisfacer* el trait: `assert_reservation_store_conformance<S: OperationReservationStore>`
(`testkit/src/reservation_conformance.rs:963-968`) y cualquier anidamiento `Arc<Arc<dyn _>>`. Sin
ella, esa instanciación genérica no compila; con un reenvío que omita `is_durable`, compila y
reporta `false` en silencio. Esa es la regresión que la prueba debe fijar.

### AD-3: La compuerta de producción se dispara por **registro**, no por `IdempotencyEnforcementMode::MandatoryKey`

**Elección**: `validate_operation_reservation_profile()` verifica *si y solo si* hay un almacén de
reservas registrado; un almacén registrado bajo `Profile::Production` DEBE ser durable. La
ausencia de almacén **no** es una falla de producción y **no** se vuelve a verificar aquí.

**Alternativas consideradas**: (i) disparar por `MandatoryKey`, en espejo de
`validate_effect_store_profile` ("hay ejecutores registrados ⇒ el almacén de efectos debe ser
durable"); (ii) `is_some_and(|s| s.is_durable())`, en espejo de
`validate_read_side_claim_profile`, que convierte la ausencia misma en falla.

**Fundamento — qué convención existente es esta, leída del código y no supuesta**: los dos
encuadres presentes en el árbol se reducen a una sola regla: *la compuerta se dispara cuando la
propia configuración de la composición muestra que la capacidad efectivamente se ejerce.*

- `validate_effect_store_profile` (`builder.rs:877-896`) está condicionada a ejecutores
  registrados porque "sin ninguno registrado no se construye almacén de efectos en absoluto".
- `validate_read_side_progress_profile` (`:905-917`) se dispara por el registro mismo — "el
  registro es en sí la señal visible en la composición de que esta proyección tiene un par de
  progreso que vale la pena gobernar".
- `validate_read_side_claim_profile` (`:927-945`) se dispara por el registro de progreso porque un
  servicio de solo comandos nunca reclama.

Para este puerto el código ya responde qué señal aplica, y lo responde sobre *este mismo almacén*.
`build()` construye la `ReservationConfig` a partir de `self.idempotency_reservation_store`
**con independencia del modo** (`:1123-1151`), y la decisión del contribuidor de salud
inmediatamente encima lo enuncia con todas las letras (`:1099-1106`): *"Keyed on the store being
present, not on the enforcement mode. A `Compatibility` runtime that did register one is still
dispatching through it, so it is still a real dependency and is checked."* Un almacén registrado
se ejerce; por lo tanto el registro es el disparador, exactamente como en
`validate_read_side_progress_profile`. La alternativa (i) dejaría un agujero real:
`Compatibility` + `Production` + almacén volátil es una composición que reserva sobre
almacenamiento volátil y sería aceptada en silencio.

La alternativa (ii) se rechaza porque `validate_idempotency` (`:846-855`) ya es dueña de
"`MandatoryKey` ⇒ DEBE haber un almacén registrado" y corre primero tanto desde `build()` como
desde `try_build()`. Hacer que la ausencia falle también aquí crearía la segunda definición
paralela de una misma regla que PROD-014A existe para evitar.

**Respuesta a la pregunta abierta, enunciada para `sdd-verify`**: la compuerta se dispara cuando
hay un almacén de reservas registrado. **No** exige además `MandatoryKey`.

### AD-4: Una sola sentencia condicional por transición de estado — ninguna transacción multi-sentencia abarca una lectura y su escritura dependiente

**Elección**: toda mutación es una única sentencia SQL condicional cuyo `WHERE` carga toda la
verificación. Stoolap ejecuta un `Database::execute` desnudo como su propia unidad atómica (la
forma que `StoolapEffectStore` usa en todo el módulo y prueba en su conformidad de concurrencia).
`db.begin()` **no se usa en ninguna parte** de este almacén.

**Fundamento**: la unidad de atomicidad que cierra la carrera de comprobar-y-actuar aquí es el
predicado, no una transacción — por eso mismo `PostgresOperationReservationStore::mutate_owned`
(`persistence/src/postgres/reservation.rs:565-605`) tampoco necesita transacción: *"Both live in
the `WHERE` clause, so verification and mutation are one statement: a separate read-then-write
would leave a window."* `StoolapSnapshotStore::save_snapshot` abre una transacción solo porque
debe hacer SELECT-y-luego-(INSERT-o-UPDATE): dos sentencias que deben ser una unidad. Ninguna
transición de aquí tiene esa forma.

**Rechazado explícitamente y a señalar en revisión**: cualquier secuencia de
`SELECT` → comparación en Rust → `UPDATE`/`DELETE` incondicional. Toda lectura de `reserve` que
sigue se usa solo para *clasificar* y para *calcular* el siguiente token; la escritura posterior
vuelve a afirmar en su propio `WHERE` cada valor que leyó, de modo que una lectura obsoleta no
puede producir una escritura equivocada: produce `affected == 0`, que luego se relee o se reporta.

**Contrato de transición por método** (este es el contrato literal de implementación):

| Método | La única sentencia atómica | Resultado ante carrera |
|---|---|---|
| `reserve` / fresca | `INSERT … VALUES (…, 'in_progress') ON CONFLICT (tenant_id, operation_key) DO NOTHING` | `affected==1` ⇒ `Fresh`; `0` ⇒ se sigue a clasificar |
| `reserve` / toma de control | `UPDATE … SET owner_id=$, fencing_token=$next, lease_until=$ WHERE tenant_id=$ AND operation_key=$ AND state='in_progress' AND fencing_token=$displaced AND lease_until <= $now` | `affected==1` ⇒ `TakenOver`; `0` ⇒ re-`SELECT` ⇒ `OwnedInProgress` / `OtherInProgress` |
| `renew` | `UPDATE … SET lease_until=$ WHERE tenant_id=$ AND operation_key=$ AND owner_id=$ AND fencing_token=$ AND state='in_progress' AND lease_until > $now` | `affected==0` ⇒ `StaleOwner` |
| `complete` | `UPDATE … SET state='completed', completed_at=$now, response=$b64 WHERE …los mismos cinco predicados…` | `affected==0` ⇒ `StaleOwner` |
| `abandon` | `DELETE FROM … WHERE …los mismos cinco predicados…` | `affected==0` ⇒ `StaleOwner` |
| `purge_completed_before` | por fila: `DELETE … WHERE tenant_id=$ AND operation_key=$ AND state='completed' AND completed_at < $cutoff` | suma de `affected`; una fila recreada entre la selección y el borrado no coincide |

### AD-5: Una carrera MVCC perdida se traduce a `Backend`, nunca a `StaleOwner`

**Elección**: en una mutación, `affected == 0` ⇒ `ReservationError::StaleOwner`. Un error crudo
para el que `stoolap_common::is_write_conflict` devuelva `true` (`UniqueConstraint`,
`TransactionAborted`, `LockAcquisitionFailed`, `DatabaseLocked`, el mensaje fijado
`"uncommitted changes from transaction"`) ⇒ `ReservationError::Backend("…; reintentar")`. Nunca
continuar en silencio.

**Alternativas consideradas**: traducir los conflictos de escritura a `StaleOwner`.

**Fundamento**: `StaleOwner` es un veredicto permanente sobre el que quien llama actúa cediendo su
lease. Un `DatabaseLocked` es transitorio y no dice nada sobre la propiedad; reportarlo como
`StaleOwner` descartaría un lease que quien llama todavía sostiene legítimamente. `affected == 0`
es la única señal de que la sentencia realmente corrió y no coincidió con nada — que es
exactamente "no es tuyo / no es ese token / ya no es válido", los tres casos que el puerto colapsa
en `StaleOwner`. El puerto no tiene variante transitoria, así que `Backend` lleva la indicación de
reintento en su mensaje, como ya hace Postgres (`reservation.rs:246-250`, `345-350`).

### AD-6: La expiración se lee de un `Clock` inyectado, exactamente como hacen ambas implementaciones existentes

**Elección**: `StoolapOperationReservationStore::open(path, clock: Arc<dyn ego_domain::Clock>)`;
toda decisión de expiración lee `clock.now()`, nunca un `now()` de SQL. Agrega `ego-domain` como
dependencia **opcional** de `ego-persistence-stoolap` (legal por capas:
`infrastructure → domain`; sin ciclo — `ego-domain → ego-persistence-api` únicamente).

**Fundamento**: no es una preferencia, es un requisito del arnés compartido.
`assert_reservation_store_conformance` toma una fábrica que devuelve `(S, Arc<TestClock>)` y
posiciona el reloj para gobernar la expiración de forma determinista. Un almacén que lea el reloj
de la base de datos no puede pasarlo. `Clock` vive en `ego-domain`, no en `ego-persistence-api`
(`crates/domain/src/time/clock.rs:24`); `ego-persistence-memory` ya carga esa misma arista de un
solo import por la misma razón.

El alcance de un solo proceso vuelve inaplicable aquí la salvedad de desfase horario que Postgres
documenta (`reservation.rs:3-34`): todos los dueños leen un mismo reloj de proceso.

### AD-7: Una feature nueva `operation-reservation` de Cargo, no la reutilización de `event-sourcing`

**Elección**: `operation-reservation = ["dep:tokio", "dep:async-trait", "dep:chrono", "dep:ego-domain", "dep:base64"]`.
Módulo en `src/operation/reservation.rs` tras `#[cfg(feature = "operation-reservation")]`.

**Alternativas consideradas**: reutilizar `event-sourcing` (mismo conjunto tokio/async-trait/chrono).

**Fundamento — desde el Cargo.toml real, no por supuesto**: el comentario de la feature existente
(`Cargo.toml:11-15`) enuncia su propósito: *"Optional so `Repository<A>`/`Snapshot` consumers of
this crate gain no new dependency."* Este almacén necesita dos dependencias que `event-sourcing`
no aporta — `ego-domain` (AD-6) y `base64` (AD-8). Agregarlas a `event-sourcing` entregaría una
arista de crate nueva a todo consumidor de event sourcing por un almacén que no usa, anulando la
razón declarada de existir de la feature. Además, un almacén de reservas no es event sourcing; el
nombre mentiría. Por eso `ego-domain` y `base64` se declaran `optional = true` y se habilitan solo
aquí.

### AD-8: Ubicación del módulo en `src/operation/reservation.rs`; `base64` para la carga de respuesta

**Elección**: `src/operation/mod.rs` + `src/operation/reservation.rs`, exportados desde la raíz del
crate según `lib.rs:13-17`. Los bytes de `StoredServiceResponse` se guardan codificados en base64
en una columna `TEXT`.

**Fundamento**: tanto el puerto (`persistence-api/src/operation/reservation.rs`) como el adaptador
en memoria (`persistence-memory/src/operation/reservation.rs`) archivan esta capacidad bajo
`operation/`; `persistence/` y `event_sourcing/` son las familias equivocadas. Para la carga, el
`core::Value` de Stoolap no tiene variante binaria — exactamente el problema de dialecto que
`StoolapEffectStore` ya resolvió con base64 TEXT
(`effect-store/src/stoolap/mod.rs:12-14,51-52`); reutilizar esa elección evita inventar una
segunda convención de codificación para un mismo problema.

### AD-9: `sync=full` se verifica con el analizador estricto, y los dos sitios de S2 migran a él

**Elección**: promover `dsn_declares_sync_full` (el analizador de cadena de consulta de
`effect-store/src/stoolap/mod.rs:187-191`) a `persistence/stoolap_common.rs`; usarlo en el
`open()` y en el `is_durable()` del almacén nuevo, y cambiar `snapshot.rs:74` y
`event_store.rs:213,263` de `db.dsn().contains("sync=full")` a él.

**Alternativas consideradas**: usar el `contains` ingenuo por consistencia con S2; agregar el
analizador estricto solo para el almacén nuevo.

**Fundamento**: la forma ingenua coincide con una *ruta* que contenga el texto
(`/data/no_sync=full/db`) — el defecto que STOOLAP-EFFECT-01 ya corrigió una vez. Enviar a
sabiendas la forma débil no es opción; enviar la estricta en solo uno de tres almacenes deja a
este crate con dos verificaciones de durabilidad, que es la "segunda forma de expresar
durabilidad" que la propuesta deja fuera de alcance. **Acotado y nombrado para que no se lea como
desborde de alcance**: dos sitios de llamada, cuatro líneas, sin cambio de comportamiento para
ningún DSN que produzca `dsn_for` (es su único productor).

### AD-10: `token_for_storage` / `token_from_storage` se reimplementan localmente, no se comparten

**Elección**: duplicar las dos guardas i64↔`FencingToken` (rechazar `raw <= 0`;
`FencingExhausted` en vez de un cast sin verificar) dentro del módulo nuevo.

**Fundamento**: los originales son `pub(crate)` en `ego-persistence` y compartirlos forzaría una
arista `ego-persistence-stoolap → ego-persistence` por veinte líneas — el mismo canje que
`StoolapEffectStore` ya hizo con `dsn_for` (`effect-store/src/stoolap/mod.rs:161-171`). La
semántica DEBE coincidir exactamente, incluido el rechazo del cero.

## Flujo de datos

```
llamador ──reserve(req)──▶ StoolapOperationReservationStore
                               │  (parámetros en propiedad, antes del límite)
                               ▼
                         spawn_blocking ──▶ Database (handle clonado, un motor compartido)
                               │                    │
                               │        tabla operation_reservations (WAL sync=full)
                               ▼
                      ReservationOutcome ◀── clasificar(affected, fila, clock.now())
```

### Secuencia: `reserve` — fresca, reejecución, conflicto y toma de control

```
Dueño-B          Store(async)      spawn_blocking       motor Stoolap        Reloj
   │                  │                   │                    │               │
   │ reserve(req) ──▶ │                   │                    │               │
   │                  │ params ──────────▶│                    │               │
   │                  │                   │ INSERT … ON CONFLICT DO NOTHING    │
   │                  │                   │───────────────────▶│               │
   │                  │                   │◀── affected = 1 ───│               │
   │◀── Fresh(lease) ─┤                   │                    │               │
   │                  │                   │                    │               │
   │  ── si no: affected = 0 ──▶ la fila ya existe ───────────────────────────  │
   │                  │                   │ SELECT fingerprint, owner_id,      │
   │                  │                   │        fencing_token, lease_until, │
   │                  │                   │        state, response             │
   │                  │                   │───────────────────▶│               │
   │                  │                   │◀────── fila ───────│               │
   │                  │                   │                                    │
   │                  │      fingerprint != req  ⇒ Conflict (se verifica PRIMERO)│
   │                  │      state = 'completed' ⇒ Succeeded(decode(response))  │
   │                  │                   │                    │               │
   │                  │                   │ now = clock.now()  │◀──────────────│
   │                  │                   │                    │               │
   │                  │  now <  lease_until ⇒ ¿coincide dueño? OwnedInProgress  │
   │                  │                   │                  : OtherInProgress │
   │                  │                   │                    │               │
   │                  │  now >= lease_until ⇒ next = displaced.next()?          │
   │                  │                   │  (None ⇒ FencingExhausted)          │
   │                  │                   │ UPDATE … SET owner_id, fencing_token=next,
   │                  │                   │              lease_until            │
   │                  │                   │  WHERE state='in_progress'          │
   │                  │                   │    AND fencing_token = displaced    │  ← CAS
   │                  │                   │    AND lease_until  <= now          │  ← revalida
   │                  │                   │───────────────────▶│               │
   │                  │                   │◀── affected = 1 ───│               │
   │◀ TakenOver(lease, token=next) ───────┤                    │               │
   │                  │                   │                    │               │
   │      affected = 0 ⇒ un par ganó la carrera en la ventana: re-SELECT,      │
   │      responder OwnedInProgress (este dueño se recuperó) u OtherInProgress.│
   │      Fila desaparecida ⇒ Backend("…reintentar el reserve"), nunca un      │
   │      resultado inventado. is_write_conflict(e) ⇒ Backend("…reintentar"),  │
   │      nunca StaleOwner.                                                    │
```

El Dueño-A, desplazado, llama luego a `renew`/`complete`/`abandon` con su token viejo: la única
sentencia condicional no coincide con ninguna fila (`fencing_token` ya no es igual) ⇒
`StaleOwner`, y la reserva queda demostrablemente sin modificar porque nada más que esa sentencia
podría haberla modificado.

## Cambios de archivos

| Archivo | Acción | Descripción |
|---|---|---|
| `crates/persistence-api/src/operation/reservation.rs` | Modificar | (a) `is_durable()` por defecto `false` + doc; reenvío `Arc<T>` de los 7 métodos; pruebas: impl desnuda da false, `Arc<Concrete>` como genérico `S` reenvía true (AD-2) |
| `crates/persistence-memory/src/operation/reservation.rs` | Modificar | (a) `fn is_durable(&self) -> bool { false }` explícito con una línea de doc: volátil es la respuesta honesta; prueba |
| `crates/persistence/src/postgres/reservation.rs` | Modificar | (a) sobrescribir `is_durable() -> true`; prueba |
| `crates/testkit/src/reservation.rs` | Solo revisar | Reexporta el almacén en memoria; no hay doble propio que cambiar. Confirmar que ningún otro doble implemente el puerto |
| `crates/persistence-stoolap/Cargo.toml` | Modificar | (b) deps opcionales `ego-domain`/`base64`; feature `operation-reservation`; `[[test]]` con `required-features` |
| `crates/persistence-stoolap/src/lib.rs` | Modificar | (b) `#[cfg(feature = "operation-reservation")] pub mod operation;` + `pub use` en la raíz |
| `crates/persistence-stoolap/src/operation/mod.rs` | Crear | (b) `pub mod reservation;` |
| `crates/persistence-stoolap/src/operation/reservation.rs` | Crear | (b) `StoolapOperationReservationStore` + pruebas unitarias colocadas |
| `crates/persistence-stoolap/src/persistence/stoolap_common.rs` | Modificar | (AD-9) agregar `dsn_declares_sync_full` + su prueba |
| `crates/persistence-stoolap/src/persistence/snapshot.rs`, `src/event_sourcing/event_store.rs` | Modificar | (AD-9) 3 sitios pasan al analizador estricto |
| `crates/persistence-stoolap/tests/reservation_conformance.rs` | Crear | (b) arnés compartido + prueba de durabilidad ante reapertura |
| `crates/service-sdk/src/runtime/builder.rs` | Modificar | (c) `validate_operation_reservation_profile()`; una línea en `validate_persistence_profile()`; pruebas de la matriz de compuerta |

## Interfaces / Contratos

```rust
// (a) crates/persistence-api/src/operation/reservation.rs
#[async_trait]
pub trait OperationReservationStore: Send + Sync {
    /// Si las reservas escritas por este almacén sobreviven a un reinicio de proceso.
    ///
    /// Por defecto `false`: honesto para toda implementación que no se haya hecho la
    /// pregunta. `Profile::Production` lo lee; una implementación durable lo
    /// sobrescribe a `true`.
    fn is_durable(&self) -> bool { false }
    // ... los 7 métodos existentes sin cambios ...
}

#[async_trait]
impl<T: OperationReservationStore + Send + Sync + ?Sized> OperationReservationStore
    for std::sync::Arc<T>
{
    /// **Determinante en contexto genérico** (AD-2): omitirlo hace que un
    /// `Arc<ConcreteStore>` usado como `S: OperationReservationStore` reporte el
    /// `false` por defecto del trait sin importar qué envuelva.
    fn is_durable(&self) -> bool { (**self).is_durable() }
    // reenvía reserve/renew/complete/abandon/purge_completed_before/
    // oldest_completed/probe — `oldest_completed` DEBE reenviarse, según el propio
    // puerto: "a wrapper MUST forward this rather than inherit the default".
}
```

```rust
// (c) crates/service-sdk/src/runtime/builder.rs
fn validate_persistence_profile(&self) -> Result<(), RuntimeError> {
    self.validate_effect_store_profile()?;
    self.validate_read_side_progress_profile()?;
    self.validate_read_side_claim_profile()?;
    self.validate_operation_reservation_profile()?;   // nueva, al final
    Ok(())
}

/// Bajo `Profile::Production`, un almacén de reservas **registrado** debe ser durable
/// (AD-3). El registro es en sí la señal visible en la composición de que este runtime
/// despacha a través del almacén — `build()` arma la `ReservationConfig` a partir de él
/// sin importar el modo de enforcement. La ausencia deliberadamente no se verifica aquí:
/// `validate_idempotency` ya es dueña de "MandatoryKey exige un almacén", y repetirlo
/// sería la segunda definición paralela que PROD-014A evita.
fn validate_operation_reservation_profile(&self) -> Result<(), RuntimeError> {
    let Some(store) = self.idempotency_reservation_store.as_ref() else {
        return Ok(());
    };
    persistent_entity::profile::require_durably_configured(
        self.profile,
        store.is_durable(),
        "durable operation reservation store (OperationReservationStore)",
        "AppBuilder::operation_reservation_store(store) (or \
         RuntimeBuilder::with_operation_reservation_store(..)), passing a store whose \
         is_durable() returns true",
    )?;
    Ok(())
}
```

```sql
-- (b) creada por open(); UNIQUE, nunca PRIMARY KEY: Stoolap analiza una PRIMARY KEY
-- compuesta a nivel de tabla pero NO la impone, mientras que UNIQUE sí se impone y es
-- contra lo que coincide ON CONFLICT (effect-store/src/stoolap/mod.rs:228-236).
CREATE TABLE IF NOT EXISTS operation_reservations (
    tenant_id     TEXT      NOT NULL,   -- '' = alcance sistémico (stoolap_common::encode_tenant)
    operation_key TEXT      NOT NULL,
    fingerprint   TEXT      NOT NULL,
    owner_id      TEXT      NOT NULL,
    fencing_token INTEGER   NOT NULL,   -- i64; token_for_storage/token_from_storage lo custodian
    lease_until   TIMESTAMP NOT NULL,
    state         TEXT      NOT NULL,   -- 'in_progress' | 'completed'
    completed_at  TIMESTAMP,
    response      TEXT,                 -- base64; NULL mientras está in_progress
    UNIQUE (tenant_id, operation_key)
)
```

```rust
// (b) constructor y durabilidad
impl StoolapOperationReservationStore {
    pub async fn open(path: &Path, clock: Arc<dyn Clock>) -> Result<Self, ReservationError>;
    async fn run_blocking<F, R>(&self, f: F) -> Result<R, ReservationError> where /* spawn_blocking */;
}
impl OperationReservationStore for StoolapOperationReservationStore {
    fn is_durable(&self) -> bool { dsn_declares_sync_full(self.db.dsn()) }
    // probe(): `SELECT 1 FROM operation_reservations LIMIT 1`, fila descartada — solo
    //          lectura, y prueba que el esquema existe, no solo que el motor responde.
    // oldest_completed(): `SELECT MIN(completed_at) … WHERE state='completed'`;
    //          NULL ⇒ Empty (una respuesta real), nunca Unsupported.
}
```

**Centinela de tenant**: `stoolap_common::encode_tenant` mapea `None` a `""`. Elegido por sobre el
`tenant_id IS NOT DISTINCT FROM $1` de Postgres porque este crate ya estableció el centinela para
evitar la semántica de comparación con NULL de SQL, y `TenantId::new("")` es rechazado, así que
ningún tenant real puede colisionar.

**Restricción de dialecto en la purga**: `DELETE … WHERE col IN (SELECT … LIMIT n)` borra **cero**
filas en silencio sobre Stoolap 0.4.0 (`effect-store/src/stoolap/mod.rs:292-301`). Por eso
`purge_completed_before` debe hacer `SELECT tenant_id, operation_key … WHERE state='completed' AND
completed_at < $cutoff LIMIT $batch` y luego borrar cada fila con su propio predicado de igualdad
**revalidando la elegibilidad** (tabla de AD-4). Revalidar es un endurecimiento deliberado por
sobre el borrado por id de `run_retention`: una clave de reserva es recreable tras `abandon`, así
que una coincidencia sola por clave podría borrar una reserva *nueva*. Sin `ORDER BY`: la
selección dentro de un lote está fuera del contrato.

## Alcance de concurrencia — qué se afirma y qué no

| Escenario | Soportado | Base |
|---|---|---|
| Mismo proceso, muchas tareas async, una instancia del almacén | **Sí** | Toda mutación es una sentencia condicional bajo el MVCC de Stoolap (AD-4); `spawn_blocking` sobre handles `Database` clonados que comparten un motor — la misma base del `concurrent_local_safe: true` de `StoolapEffectStore` |
| Mismo proceso, dos instancias de runtime / dos instancias del almacén en la misma ruta | **Sí** | El registro global de proceso de Stoolap comparte un motor vivo por DSN mientras haya un handle vivo (`effect-store/src/stoolap/mod.rs:200-215`); todos los dueños leen un mismo reloj de proceso, sin desfase de expiración. Debe *probarse*, no suponerse (TS-4) |
| Varios procesos del SO sobre el mismo archivo | **No soportado, no probado** | Dos motores sobre un archivo; nada en el árbol establece bloqueo entre procesos. No se hace ninguna afirmación |
| Multi-nodo | **No soportado** | El mismo precedente de honestidad que el `multi_node_safe: false` de `StoolapEffectStore` (`effect-store/tests/conformance.rs:296-302`) |

El fencing acota lo que puede acotar: vuelve autoritativo el *resultado de la reserva*. No cancela
un efecto externo ya emitido por un dueño desplazado — el mismo límite que enuncia Postgres
(`reservation.rs:22-31`). Decirlo en la doc del módulo; no suavizarlo.

## Estrategia de pruebas

| Capa | Qué probar | Enfoque |
|---|---|---|
| Unitaria — puerto (a) | La impl desnuda hereda `false`; `Arc<Concrete>` como genérico `S` reporta `true`; `Arc` reenvía los 7 métodos incl. `oldest_completed` | `#[cfg(test)]` colocado, en espejo de `dedup.rs:116-175`. La prueba de durabilidad sobre `Arc` DEBE usar un genérico `S`, no `Arc<dyn _>` (AD-2) |
| Unitaria — implementadores (a) | En memoria reporta `false`; Postgres reporta `true` | Colocadas; la de Postgres no necesita pool (`is_durable` es pura) |
| Unitaria — Stoolap (b) | `open()` rechaza un motor sin `sync=full`; `is_durable()` verdadero; `dsn_declares_sync_full` rechaza una ruta que contenga el texto | Colocadas, `tempfile` por prueba + `stoolap::test_failpoints::FailpointGuard`, como hace `snapshot.rs:179-197` |
| Integración (b) | `assert_reservation_store_conformance` contra un almacén Stoolap + `TestClock` | `tests/reservation_conformance.rs`, `required-features = ["operation-reservation"]`. El mismo arnés que pasa el almacén en memoria — sin una segunda copia del contrato |
| Integración (b) | Durabilidad ante reapertura: reservar → soltar el almacén → reabrir la misma ruta → dueño, token de fencing, cota de lease y alcance de tenant intactos; un fence obsoleto sigue siendo rechazado tras reabrir | Prueba nueva; la forma de fábrica que usa `StoolapDurableStoreFactory` |
| Integración (b) | TS-4 mismo proceso, dos instancias del almacén en una ruta: A reserva, el reloj avanza más allá del lease, B (instancia separada) toma el control con un token estrictamente mayor, y luego el fence de A falla con `StaleOwner` | Prueba la fila de la tabla anterior en vez de afirmarla |
| Integración (b) | Aislamiento de tenants: la misma clave bajo tenant A / tenant B / sistémico son tres reservas | Explícita — el centinela es donde esto podría romperse en silencio |
| Integración (b) | Purga: se respeta la cota del lote, nunca se quita una en progreso, el conteo son las filas realmente borradas, drenaje en llamadas sucesivas | Cubierto por el grupo de purga del arnés; sumar una prueba específica de Stoolap que demuestre que el lote en dos pasos efectivamente borra (protege contra la trampa de dialecto `IN (SELECT … LIMIT)`) |
| Unitaria — compuerta (c) | Matriz `{Dev, Production} × {sin almacén, almacén volátil, almacén durable}` — solo `Production` + volátil rechaza | Pruebas en `builder.rs`, clonando la forma de `validate_read_side_claim_profile_matrix` con `compat()` y un `StubReservationStore(bool)` |
| Unitaria — compuerta (c) | `Production` + almacén volátil bajo modo **`Compatibility`** igual rechaza | La prueba que distingue AD-3 de la alternativa rechazada disparada por `MandatoryKey`. Sin ella la decisión queda sin fijar |
| Unitaria — compuerta (c) | El rechazo nombra la capacidad y `with_operation_reservation_store` | En espejo de `validate_read_side_claim_profile_rejects_volatile_claim_store` |
| Unitaria — compuerta (c) | `build()` entra en pánico y `try_build()` devuelve el mismo rechazo | Ambas rutas, como toda compuerta hermana |

El modo TDD está activo (`openspec/config.yaml`): cada fila aterriza primero en ROJO.
`cargo test --workspace` no habilita `operation-reservation`; las pruebas del corte (b) deben
correrse además con `--features operation-reservation`, y el `--all-targets` de CI debe seguir
compilando sin ella — por eso la prueba de integración nueva lleva `required-features`
(precedente en `Cargo.toml:50-52`).

## Matriz de amenazas

No aplica — no hay ruteo, shell, subprocesos, automatización de VCS/PR, clasificación de archivos
ejecutables ni límite de integración de procesos. La única preocupación adyacente, texto
controlado por quien llama llegando a SQL, se maneja con la misma regla que enuncia Postgres: todo
valor se vincula como `$N`, nunca se interpola — una `OperationKey` la provee el cliente.

## Migración / Despliegue

Sin migración de datos. `open()` emite `CREATE TABLE IF NOT EXISTS`, así que funcionan tanto una
base nueva como una base S1/S2 existente en la misma ruta. El corte (a) es aditivo con valor por
defecto `false`, así que ningún implementador externo se rompe. El corte (b) es puramente aditivo
y queda apagado por defecto tras la feature. El corte (c) es el único cambio de comportamiento y
se revierte solo; un host que estuviera registrando un almacén de reservas volátil bajo
`Profile::Production` fallará en tiempo de construcción con un mensaje que nombra la corrección
— que es la intención.

Corte por presupuesto de revisión: (a) ≈ 150 líneas, (b) ≈ 550 líneas, (c) ≈ 200 líneas. Tres PR
apilados; (b) es el que corre riesgo de superar 400 y puede dividirse en (b1) almacén +
conformidad y (b2) pruebas de reapertura/concurrencia/tenants.

## Preguntas abiertas

- [ ] ¿Stoolap 0.4.0 soporta `MIN(completed_at)`? `MAX`+`COALESCE` está probado en el árbol
      (`event_store.rs:64-65`), `MIN` no. Si no está soportado, `oldest_completed` recurre a
      `SELECT completed_at … WHERE state='completed' ORDER BY completed_at ASC LIMIT 1` — misma
      respuesta, sin cambio de contrato. Resolver por experimento en el corte (b), no por supuesto.
- [ ] ¿Una columna `TIMESTAMP` anulable se lee de vuelta como `Option<DateTime<Utc>>` a través de
      la API de filas de Stoolap? El esquema de `effect-store` tiene timestamps anulables, así que
      la forma está establecida; confirmar el accesor exacto antes de escribir el manejo de
      `completed_at`.
- [ ] Seguimiento no bloqueante, explícitamente fuera de este cambio: nada renueva un lease de
      reserva automáticamente (doc del puerto, `reservation.rs:19-27`). Un despliegue respaldado
      por Stoolap hereda eso sin cambios; no es una brecha que este cambio introduzca.
