# Diseño: PROD-014C — Reclamo Atómico de Eventos del Lado de Lectura

> Compañero de revisión en español. La fuente de verdad es `design.md` (identificadores 1:1).
>
> **Entradas**: `proposal.md` (D-1 … D-12, IS-1 … IS-8, OOS-1 … OOS-6, R-1 … R-5, SC-1 … SC-7,
> Semántica Requerida) y `exploration.md`. D-1 … D-12 están fijadas y no se vuelven a discutir
> aquí. Este documento decide el **cómo**: mecanismo, forma del puerto, SQL, cableado de
> sesión/planificador, forma de la compuerta, esquema y forma de las pruebas.
>
> **Línea base leída**: `develop` @ `30e42ab`. Cada `archivo:línea` citado fue leído sobre esta
> línea base, no recordado.
>
> **Disciplina de redacción (R-1, SC-6)**: este documento dice *propiedad de procesamiento válida
> única*, *reclamo atómico* y *exclusión de ejecución*. Nunca dice exactamente-una-vez, seguro
> ante concurrencia, ni seguro para múltiples réplicas como propiedad lograda.

## Enfoque Técnico

Un puerto nuevo (`ReadSideClaimStore`) en `crates/persistence-api/src/read_side/claim.rs`, un
adaptador durable sobre una tabla nueva (`projection_claims`, migración `016`) y un reclamo
verificado por fencing que envuelve el cuerpo actual de `ReadSideSession::execute()`. La identidad
del reclamo es `(projection_id, tag, tenant)` — byte por byte la clave primaria de
`projection_offsets` (D-1).

La adquisición es **una sola** sentencia: `INSERT … ON CONFLICT (…) DO UPDATE … WHERE lease_until
<= now RETURNING fencing_token`. Inserta si no existe, toma el control solo si el arriendo vigente
caducó, acuña un token estrictamente mayor en la misma sentencia y, en cualquier otro caso, afecta
cero filas. No hay ventana de verificar-luego-actuar creada por este diseño, ni lógica en Rust
entre dos consultas.

El reclamo se toma **por flujo y por lote**, no por evento (R-3): `try_claim` antes de `fetch`,
`renew` inmediatamente antes de la fase de confirmación, `release` en toda ruta de salida. Un
trabajador rechazado devuelve `Ok(None)` y no invoca ni `fetch` ni el manejador.

`OffsetStore` y `DedupStore` se consumen sin cambios (D-2, D-11). La firma pública del rasgo
`TagScheduler` no cambia.

---

## La Comparación de Mecanismos (A–E)

Los cinco candidatos se evalúan contra las siete propiedades requeridas. El criterio de decisión
es el de la propuesta: **el mecanismo mínimo que satisfaga rigurosamente todas ellas.**

### La propiedad que decide: seguridad ante trabajador obsoleto

Todo lo demás lo satisface más de un candidato. Esta no, y la razón es estructural, no de grado.

`write_offset` es un upsert simple con "gana la última escritura" por contrato
(`crates/persistence/src/postgres/read_side_offset.rs:5-14`, PROD-014B AD-3). Un trabajador
obsoleto que se reanuda y escribe su propio `Offset::sequence(v_A)` menor sobre el `v_B > v_A` de
un propietario vivo no solo desperdicia un `fetch`. El rango re-leído `v_A+1 … v_B` ya está
marcado en `projection_dedup`, así que `unique_events` queda vacío y `execute()` retorna en
`session.rs:130-132` — **antes** de llegar a `write_offset`. El offset queda entonces retrocedido
y solo vuelve a avanzar cuando aparecen eventos nuevos más allá de la ventana del lote; con más de
`batch_size` eventos en el hueco, la proyección se estanca indefinidamente. Es exactamente la
falla que describe el comentario de `session.rs:81-87` para un offset que no reanuda, alcanzada
desde el otro lado.

Por lo tanto, un mecanismo debe permitir que un trabajador **le pregunte a la base de datos si
sigue siendo el propietario y reciba un no**. Esa pregunta solo es respondible si el trabajador
porta algo que la base de datos pueda comparar contra la fila actual.

| | (A) `FOR UPDATE` | (B) advisory lock | (C) tabla de reclamo + fencing | (E) `FOR UPDATE SKIP LOCKED` |
|---|---|---|---|---|
| Exclusión mutua | Sí, mientras la tx está abierta | Sí, mientras la sesión lo retiene | Sí, por la PK y el `WHERE` | Sí, igual que (A) |
| Adquisición atómica (una primitiva) | Necesita `INSERT … ON CONFLICT` primero para materializar una fila que bloquear, y luego `FOR UPDATE` — **dos** sentencias | Una llamada `pg_advisory_lock` | **Una** `INSERT … ON CONFLICT DO UPDATE … WHERE … RETURNING` | Las mismas dos sentencias que (A) |
| Recuperación ante caída | Excelente — muere con la tx, sin arriendo ni reloj | Auto-liberación al cerrar la conexión, pero "conexión cerrada" ≠ "trabajador detenido"; recolectar un backend medio abierto es un ajuste de TCP/keepalive, no un límite configurado | Caducidad de arriendo contra un `Clock` inyectado (D-4) | Igual que (A) |
| **Seguridad ante trabajador obsoleto (fencing)** | **No — ver abajo** | **No — ver abajo** | **Sí** — el trabajador porta `(owner_id, fencing_token)` y cada mutación reverifica la tripleta completa en su propio `WHERE` | **No** — hereda (A) |
| Seguridad multi-nodo | Sí (sin estado en memoria) | Sí | Sí | Sí |
| Preservación del orden | Sí (el reclamo es por flujo) | Sí | Sí | Sí |
| Semántica de reintento bajo contención | Bloquea en cabeza de línea, o `NOWAIT` para rechazar | Bloquea, o `try_advisory_lock` para rechazar | Rechaza de inmediato: `rows_affected() == 0`, sin espera y sin retener bloqueo | Omite — pero ver abajo |
| Costo de conexiones | Fija una conexión del pool por flujo activo durante todo el lote, **incluido `handler.handle()`** (E/S de usuario arbitraria) | Igual, más una conexión dedicada fijada | Un viaje por sentencia; nada se retiene entre ellas | Igual que (A) |
| Observabilidad | `pg_locks` muestra un bloqueo de tx, no qué trabajador posee qué flujo | `pg_locks` muestra `classid`/`objid` — el **hash**, no la identidad | `SELECT * FROM projection_claims` nombra propietario, token y arriendo | Igual que (A) |

#### Por qué (A) demostrablemente no puede resolver la seguridad ante trabajador obsoleto

Un bloqueo no es un token. Cuando el bloqueo se libera — por caída, por
`idle_in_transaction_session_timeout`, por reciclaje de una conexión del pool, o por una partición
en la que el backend es recolectado — PostgreSQL lo libera mientras el futuro de Rust del
poseedor sigue detenido dentro de `handler.handle()`. Nada se lo comunica al futuro. Cuando se
reanuda y pregunta "¿sigo siendo el propietario?", lo único que puede hacer es abrir una
transacción nueva y volver a bloquear — **lo cual tiene éxito**, porque el bloqueo anterior ya no
existe. El mecanismo responde *sí* a un trabajador que ya fue reemplazado, y ese trabajador
procede a retroceder el offset.

La única forma de cerrarlo bajo (A) es ejecutar `mark_seen` y `write_offset` **dentro de la misma
transacción que sostiene el bloqueo**, lo que exige atravesar un manejador de transacción por
`OffsetStore`/`DedupStore`. `crates/domain` tiene cero conocimiento de PostgreSQL por diseño
hexagonal y ningún puerto porta un manejador de transacción (`exploration.md` §"Transactional
guarantees — none, and why"); agregarlo reabre los dos contratos archivados de PROD-014B (**D-2**)
y es precisamente la atomicidad entre tablas que **D-11** excluye. Así que (A) no es "más débil
aquí" — es inimplementable dentro de las decisiones fijadas de este cambio.

#### Por qué (B) tampoco puede, más tres fallas propias

Misma causa raíz: no existe ningún token, así que "¿sigo siendo el propietario?" vuelve a ser una
re-adquisición que tiene éxito. Agregar un token significa agregar una tabla — momento en el cual
la tabla es el mecanismo y el advisory lock es decoración. Además:

1. **Incompatibilidad con el pool.** `pg_advisory_lock` tiene alcance de *sesión*, ligado a una
   conexión de backend. `sqlx::PgPool` elige la conexión por consulta, así que el bloqueo se
   adquiere en C1 mientras la sentencia siguiente corre en C2. La corrección exige retener un
   `pool.acquire()` dedicado durante toda la vida del reclamo — el costo de fijar conexiones de
   (A), con un bloqueo que ya no muere con una transacción. `pg_advisory_xact_lock` tiene alcance
   de transacción y por lo tanto colapsa exactamente en (A), heredándolo todo.
2. **Colisiones de hash silenciosas.** Las claves son `bigint`; `(projection_id, tag, tenant)` son
   tres cadenas, así que hay que hashearlas a 64 bits. Una colisión excluye mutuamente dos flujos
   no relacionados — un estancamiento que parece una proyección ociosa.
3. **No diagnosticable.** `pg_locks` expone el hash, no la identidad, así que un operador no puede
   responder "¿qué trabajador posee `users-by-tenant:tenant-a`?" desde ninguna vista del sistema.
   La colisión (2) es, por lo tanto, invisible desde fuera.

#### Por qué (E) es una pista falsa aquí

`reservation.rs:485-501` usa `FOR UPDATE SKIP LOCKED` en `purge_completed_before`, y su propio
comentario delimita el alcance con precisión: es una garantía de **progreso**, no de seguridad —
"sin ella dos trabajadores igualmente no pueden eliminar la misma fila dos veces … Lo que provee
es que un trabajador cuyo lote podría llenarse con filas no bloqueadas lo llene", resguardado por
`purge_progress_postgres.rs`. Eso es consumidores en competencia drenando filas intercambiables.

Los flujos del lado de lectura no son intercambiables y no hay cola. `tag_provider()`
(`scheduler.rs:308`) decide qué pares `(tag, tenant)` sirve *este proceso*, y `ReadSideStore::fetch`
garantiza el orden *dentro* de un flujo. Un trabajador debe sondear su propio flujo, no la fila que
resulte estar sin bloqueo. Y como modificador de `FOR UPDATE`, (E) hereda textualmente los
problemas de vida-de-transacción y ausencia-de-token de (A).

La única propiedad útil de (E) — **rechazar en vez de bloquear** — la entrega (C) de forma nativa:
`rows_affected() == 0` rechaza sin tomar ni esperar ningún bloqueo.

#### (D) Arriendo + fencing: el cómo, no el si

D-3 ya concluyó que el fencing es obligatorio. Formalizado:

- **Forma del token** — reutilizar `FencingToken`
  (`crates/persistence-api/src/operation/reservation.rs:225-267`): `u64`, comienza en 1,
  `next()` usa `checked_add` y reporta agotamiento en vez de dar la vuelta. Toda su promesa
  documentada — "un token que dio la vuelta podría comparar igual a un fence que un propietario
  previo aún porta" — es la misma promesa que se necesita aquí (AD-3).
- **Dónde se acuña** — dentro del mismo `ON CONFLICT DO UPDATE` que reasigna la fila, como
  `projection_claims.fencing_token + 1`. Nunca leer-luego-incrementar en Rust.
- **Dónde se verifica** — en la cláusula `WHERE` de cada sentencia mutante (`renew`, `release`),
  junto a `owner_id` y la identidad completa. Verificación y mutación son una sola sentencia,
  exactamente la forma de `mutate_owned` de `reservation.rs` (`:578-605`).
- **Ante desajuste** — cero filas afectadas ⇒ `ClaimError::StaleOwner`, con la fila garantizada
  sin modificar. La sesión trata `StaleOwner` del `renew` previo a la confirmación como un aborto:
  no escribe ningún marcador de dedup ni ningún offset (AD-6).

### Decisión

**(C) — una tabla de reclamo durable y atómica con arriendo y token de fencing.** Es el único
candidato que satisface del todo la seguridad ante trabajador obsoleto, y el único cuya
adquisición es una sola sentencia. También es el único que no fija una conexión del pool a través
de código de usuario arbitrario, y el único que un operador puede consultar.

Es mínimo en el sentido que exige el criterio: **tres métodos, no cuatro.** El `complete` de
`reservation.rs` existe porque una reserva almacena una respuesta para reproducirla luego. Un
reclamo del lado de lectura no almacena nada — el offset y los marcadores de dedup son propiedad
de los otros dos puertos — así que `complete` colapsa en `release`. El ilustrativo
`try_claim`/`renew`/`complete`/`release` de la propuesta se recorta a
`try_claim`/`renew`/`release` (más `is_durable`, que lee la compuerta).

---

## Mapa de Componentes

```
crates/persistence-api/src/read_side/
├── claim.rs                          NUEVO  ReadSideClaimStore + tipos + impl Arc<T>  (AD-1..AD-4)
└── mod.rs                            MOD    `pub mod claim;`

crates/domain/src/read_side/
├── mod.rs                            MOD    re-export de `claim` con la misma forma de ruta
└── session.rs                        MOD    knob ReadSideClaiming + claim/renew/release (AD-6)

crates/persistence/src/postgres/
├── migrations/016_create_projection_claims.sql   NUEVO   (AD-8)
├── migrations.rs                     MOD    una const + una entrada de registro
├── read_side_claim.rs                NUEVO  PostgreSQLReadSideClaimStore              (AD-5)
├── reservation.rs                    MOD    `token_from_storage` → pub(crate)         (AD-3)
└── mod.rs                            MOD    un `pub use`

crates/runtime/src/read_side/scheduler.rs         MOD  knob ProjectionSpec::claims      (AD-7)
crates/service-sdk/src/runtime/builder.rs         MOD  slot + validate_read_side_claim_profile (AD-9)
crates/service-sdk/src/app/mod.rs                 MOD  AppBuilder::read_side_claims     (AD-9)
examples/reference-app/src/read_side/mod.rs       MOD  retirar la promesa de PROD-014C  (IS-8)
integration-tests/tests/infrastructure/
└── read_side_claiming_postgres.rs    NUEVO  la suite de contención                    (AD-10)

crates/persistence-api/src/read_side/{offset,dedup,store}.rs   SIN TOCAR  (D-2)
crates/persistence/src/postgres/{read_side_offset,read_side_dedup}.rs  SIN TOCAR  (D-2)
```

## Flujo de Datos

```
ReadSideSession::execute()                              PostgreSQL
──────────────────────────                              ──────────
 0  try_claim(id, owner, now+lease) ───────────────▶  INSERT INTO projection_claims …
        │                                              ON CONFLICT (pid,tag,tenant) DO UPDATE
        │                                                SET owner_id=EXCLUDED.owner_id,
        │                                                    fencing_token=…+1, lease_until=…
        │                                              WHERE projection_claims.lease_until <= $now
        │                                              RETURNING fencing_token
        ├── None (0 filas) ──▶ Ok(None). Sin fetch. Sin manejador. ◀── rechazado
        ▼ Some(fence)
 1  read_offset  ─┐
 2  fetch         ├─ sin cambios; se preserva event_version ascendente por (tenant, tag)
 3  filtro dedup ─┘   porque solo quien porta el fence llega hasta ellos
        ▼
 4  handler.handle(unique_events)     ← la ventana que nombró PROD-014B AD-6, ahora dentro de un reclamo
        ▼
 5  renew(fence, now+lease) ───────────────────────▶  UPDATE … SET lease_until=$1
        │                                             WHERE pid,tag,tenant AND owner_id=$
        │                                               AND fencing_token=$ AND lease_until > $now
        ├── StaleOwner (0 filas) ──▶ aborta. Sin mark_seen. Sin write_offset. ◀── excluido por fencing
        ▼
 6  mark_seen ×n ; write_offset ; on_batch_completed   (puertos sin cambios, D-2)
        ▼
 7  release(fence) ────────────────────────────────▶  UPDATE … SET lease_until = $now
                                                       (mismo WHERE de fence completo)
                                                       → reclamable de inmediato
```

Los pasos 1-6 también corren bajo `release` en toda ruta de salida, incluida la de error (AD-6).

---

## Decisiones de Arquitectura

### AD-1 — El puerto: `ReadSideClaimStore`, cuatro métodos, en `persistence-api`

**Decisión** — `crates/persistence-api/src/read_side/claim.rs`:

```rust
/// The capability port through which one worker obtains single valid
/// processing ownership of a (projection_id, tag, tenant) stream.
///
/// Every mutating call (`renew`, `release`) MUST verify the full
/// `claim_id + owner_id + fencing_token` triple inside the same statement
/// that mutates. A caller whose claim was taken over receives
/// `ClaimError::StaleOwner` and its call MUST leave the claim unmodified.
#[async_trait::async_trait]
pub trait ReadSideClaimStore: Send + Sync {
    /// Whether claims obtained through this store survive a process restart.
    /// Defaults to `false`, mirroring `OffsetStore::is_durable`
    /// (`offset.rs:62-64`). `Profile::Production` reads this (IS-6).
    fn is_durable(&self) -> bool {
        false
    }

    /// Obtains the claim, or reports that a live claim already holds it.
    ///
    /// `Ok(None)` is a refusal, not a failure: another worker holds an
    /// unexpired lease. The caller MUST NOT fetch or invoke the handler.
    /// `Ok(Some(fence))` is granted, whether fresh or taken over from a
    /// lapsed owner; the fence carries a strictly greater token than any
    /// this identity previously issued.
    ///
    /// `lease_until` is computed by the caller (`clock.now() + configured
    /// lease`), never by the store — `ReserveRequest::lease_until`'s rule
    /// (`operation/reservation.rs:332-335`) and D-4.
    async fn try_claim(
        &self,
        claim_id: &ClaimId,
        owner_id: &OwnerId,
        lease_until: DateTime<Utc>,
    ) -> Result<Option<ClaimFence>, ClaimError>;

    /// Extends an owned, still-valid claim to `lease_until`.
    ///
    /// MUST reject a stale fence AND an already-lapsed lease with
    /// `StaleOwner`, leaving the claim unmodified — a lapsed holder
    /// resurrecting its claim would defeat a takeover that was already
    /// legitimate (`operation/reservation.rs:74-81`).
    async fn renew(
        &self,
        fence: &ClaimFence,
        lease_until: DateTime<Utc>,
    ) -> Result<(), ClaimError>;

    /// Releases an owned, still-valid claim, making the stream immediately
    /// claimable without waiting for expiry. Same fence rule as `renew`.
    async fn release(&self, fence: &ClaimFence) -> Result<(), ClaimError>;
}
```

**Criterios**: (a) `#[async_trait]` y un `fn is_durable() -> bool { false }` por defecto son
exactamente las convenciones de `OffsetStore` (`offset.rs:54-64`) — un `false` por defecto es
honesto para toda implementación que no se haya planteado la pregunta, y la compuerta lo lee;
(b) `try_claim` no recibe fence porque todavía no hay nada que probar, y `renew`/`release` no
reciben *nada más* que el fence porque el fence porta la identidad; (c) tres verbos, según la
Decisión anterior.

**Rechazado — un enum `ClaimOutcome`.** `ReservationOutcome` necesita seis variantes porque una
reserva porta una huella y una respuesta almacenada, así que debe distinguir `Conflict`,
`Succeeded`, `OwnedInProgress` y `OtherInProgress`. Un reclamo es binario: concedido o rechazado.
`Option<ClaimFence>` dice exactamente eso sin inventar nombres. `Fresh` vs `TakenOver` tampoco se
distingue — ningún requisito de este cambio lo lee, y la toma de control es directamente
observable en la fila como `fencing_token > 1` (SC-2 lo verifica ahí).

**Rechazado — un método en `OffsetStore`.** D-2, fijado.

### AD-2 — Tipos: `ClaimId` y `ClaimFence` son nuevos; `OwnerId` y `FencingToken` se reutilizan

**Decisión**:

```rust
/// The claim identity — exactly `projection_offsets`' primary key (D-1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClaimId {
    pub projection_id: String,
    pub tag: EventTag,
    pub tenant: String,
}

/// The full verification triple every mutating call presents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimFence {
    pub claim_id: ClaimId,
    pub owner_id: OwnerId,
    pub fencing_token: FencingToken,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClaimError {
    /// The presented fence no longer matches the current claim — typically
    /// because the claim was taken over. The claim is unmodified.
    #[error("stale owner: the presented fence no longer matches the current claim")]
    StaleOwner,
    /// No strictly greater token can be minted, so takeover cannot proceed
    /// safely. Unreachable in practice; represented rather than wrapped.
    #[error("fencing token sequence exhausted for this claim")]
    FencingExhausted,
    #[error("transient claim store error: {0}")]
    Transient(String),
    #[error("fatal claim store error: {0}")]
    Fatal(String),
}
```

**Criterios**: `OwnerId` (`operation/reservation.rs:203-217`) se documenta a sí mismo como
"identifica la instancia llamadora que porta (o intenta portar) un arriendo" — nada específico de
operaciones. Lo mismo con `FencingToken`, cuya semántica de `next()` verificado/agotamiento y sus
pruebas unitarias (`:489-521`) son precisamente lo que este cambio necesita y que de otro modo se
volvería a derivar. Ambos viven en `ego-persistence-api`, la misma caja que el puerto nuevo, así
que la reutilización no cruza ninguna frontera.

**Rechazado — reutilizar `ReservationError`.** Su `Backend(String)` colapsa la división
`Transient`/`Fatal` que es la convención del lado de lectura en `OffsetStoreError`
(`offset.rs:42-49`) y `DedupStoreError`, y que el predicado `is_fatal` de PROD-014B AD-8 existe
para calcular. El helper se reutiliza (AD-5); el tipo no.

**Rechazado — duplicar `OwnerId`/`FencingToken` con nombres del lado de lectura.** Dos tipos de
token monótono idénticos con dos copias del argumento de agotamiento es estrictamente más
superficie diciendo estrictamente menos.

### AD-3 — Impl general de reenvío para `Arc<T>`, con `is_durable()` reenviado explícitamente

**Decisión**: la misma impl general que porta `OffsetStore` (`offset.rs:86-119`):

```rust
#[async_trait::async_trait]
impl<T: ReadSideClaimStore + Send + Sync + ?Sized> ReadSideClaimStore for std::sync::Arc<T> {
    /// **Load-bearing.** Omitting this silently inherits the trait's `false`
    /// default, so every registered store would be classified volatile no
    /// matter what the host wrapped — the gate would refuse a correct durable
    /// composition and pass nothing (PROD-014A EC-2).
    fn is_durable(&self) -> bool {
        (**self).is_durable()
    }
    // … tres cuerpos de reenvío
}
```

**Criterios**: D-2 cita este requisito directamente. La raíz de composición porta el store como
`Arc<dyn ReadSideClaimStore + Send + Sync>` y entrega ese mismo valor a `ProjectionSpec`, así que
el valor registrado y el valor lanzado deben ser el mismo valor. Fijado por la misma prueba trampa
`arc_forwards_is_durable` que porta `offset.rs:189-197`.

También en esta AD: el `token_from_storage` privado de
`crates/persistence/src/postgres/reservation.rs` (`:124-138`) pasa a `pub(crate)`. Son seis líneas
cuyo contenido íntegro es el argumento de que un token almacenado debe ser positivo y que
`u64::try_from` acepta el cero — volver a derivarlo en un segundo adaptador es cómo las dos copias
divergen.

### AD-4 — El rechazo de `try_claim` es `Ok(None)`, y la sesión también lo convierte en `Ok(None)`

**Decisión**: un reclamo rechazado no es un `ProjectionError`. `ReadSideSession::execute()` ya
devuelve `Ok(None)` para "nada avanzó en este tick" (`session.rs:112-114`, `:130-132`), y un
rechazo es exactamente eso.

**Criterios**: en un despliegue de dos réplicas, la réplica no propietaria es rechazada en **cada**
tick para **cada** flujo que no posee. Clasificar eso como error dispararía `on_error`
(`scheduler.rs:321`) a la frecuencia de sondeo, en la mayoría de las réplicas, de forma permanente
— convirtiendo el estado estable normal en una inundación de logs. La fila de reclamo es en cambio
la superficie de observabilidad: nombra al propietario, el token y el arriendo (AD-8).

**Rechazado — un método nuevo en `ProgressReporter` o una variante nueva de `ProjectionError`.**
Ambos amplían un puerto ya publicado para reportar un no-evento, y ninguno lo exige algún criterio
de éxito.

### AD-5 — El adaptador: una sentencia por operación, `is_fatal` reutilizado

**Decisión** — `crates/persistence/src/postgres/read_side_claim.rs`,
`PostgreSQLReadSideClaimStore { pool: PgPool, clock: Arc<dyn Clock> }`, `Debug` manual que imprime
solo el pool (`reservation.rs:75-81`), `is_durable() -> true`.

`try_claim` — **una** sentencia, todo el mecanismo:

```rust
let token: Option<i64> = sqlx::query_scalar(
    r#"INSERT INTO projection_claims
           (projection_id, tag, tenant, owner_id, fencing_token, lease_until, claimed_at)
       VALUES ($1, $2, $3, $4, 1, $5, NOW())
       ON CONFLICT (projection_id, tag, tenant) DO UPDATE
          SET owner_id      = EXCLUDED.owner_id,
              fencing_token = projection_claims.fencing_token + 1,
              lease_until   = EXCLUDED.lease_until,
              claimed_at    = NOW()
        WHERE projection_claims.lease_until <= $6
       RETURNING fencing_token"#,
)
.bind(&claim_id.projection_id).bind(claim_id.tag.value()).bind(&claim_id.tenant)
.bind(owner_id.as_str()).bind(lease_until).bind(self.clock.now())
.fetch_optional(&self.pool).await.map_err(claim_error)?;
```

Comportamiento, caso por caso:

| Estado de la fila | Resultado |
|---|---|
| Ausente | Ruta INSERT, token `1`, `Some(fence)` — fresco |
| Presente, `lease_until <= now` | Se dispara DO UPDATE: nuevo propietario, token estrictamente `+1`, `Some(fence)` — toma de control |
| Presente, `lease_until > now` | El `WHERE` del DO UPDATE lo suprime; cero filas; `RETURNING` no devuelve nada → `None` — rechazado |
| Dos inserts concurrentes | uno gana el índice único; la ruta `ON CONFLICT` del otro espera ese bloqueo de fila y, bajo READ COMMITTED, reevalúa su `WHERE` contra la **fila ganadora ya confirmada**, ve un arriendo vivo y es rechazado |

Esa última fila es el mismo razonamiento que declara `reservation.rs:283-294` para su predicado de
toma de control — "un llamador que esperó el bloqueo de fila se juzga contra la fila que existe, no
contra la fila que recuerda" — solo que aquí ocurre *dentro de una sola sentencia* en vez de entre
una lectura y una escritura, así que este diseño no tiene ninguna ventana propia que defender.

`renew` y `release` comparten un helper privado con forma de `mutate_owned`, estructuralmente
idéntico a `reservation.rs:578-605`:

```sql
-- renew                                        -- release
UPDATE projection_claims                        UPDATE projection_claims
   SET lease_until = $1                            SET lease_until = $1   -- ligado a now
 WHERE projection_id = $2 AND tag = $3           WHERE …WHERE de fence idéntico…
   AND tenant = $4 AND owner_id = $5
   AND fencing_token = $6
   AND lease_until > $7
```

Cero filas afectadas ⇒ `ClaimError::StaleOwner`, por los tres motivos "no es tuyo", "no es ese
token" y "ya no es válido", que el puerto deliberadamente no distingue
(`reservation.rs:576-577`).

**La liberación es una caducidad, nunca un `DELETE`.** Un `DELETE` haría que el siguiente
`try_claim` tomara la ruta INSERT y **reiniciara el token a 1**, rompiendo la monotonía estricta a
través de la frontera de liberación — un fence obsoleto de dos generaciones atrás podría entonces
comparar igual a uno vivo, y lo único que aún los separaría sería `owner_id`. Fijar
`lease_until = now` conserva la fila, mantiene el token estrictamente monótono durante toda la vida
de la identidad, y hace verdadero "reclamable de inmediato" mediante el mismo predicado único que
`try_claim` ya evalúa. La cardinalidad no se ve afectada: una fila por cada
`(projection_id, tag, tenant)` visto alguna vez — la misma cota que tiene `projection_offsets`
(AD-8).

**Mapeo de errores**: `claim_error` reutiliza textualmente el `pub(crate) is_fatal` de PROD-014B
AD-8 (`postgres/mod.rs`) para la división `Transient`/`Fatal`, con un código verificado antes:
SQLSTATE `22003` (`numeric_value_out_of_range`) → `ClaimError::FencingExhausted`. Como el
incremento ocurre **en SQL**, PostgreSQL lanza error en vez de dar la vuelta al tope de `BIGINT`,
así que este adaptador obtiene gratis la garantía que `reservation.rs` debe comprar con
`token_for_storage` (`:107-109`) — la diferencia deliberada entre incrementar en SQL e incrementar
en Rust. El valor leído de vuelta por `RETURNING` sigue pasando por `token_from_storage` (AD-3).

**Sin `probe()`.** El mismo razonamiento que PROD-014B AD-9: el puerto no declara método de salud,
y una migración `016` no aplicada aparece como `42P01` → `Fatal`.

### AD-6 — Dónde envuelve el reclamo al lote, y la ventana residual, declarada

**Decisión** — `crates/domain/src/read_side/session.rs`:

```rust
/// Everything a session needs to claim its stream. One optional knob, so
/// every existing `ReadSideSession::new` call site compiles unchanged;
/// `Profile::Production` is what makes it non-optional in a real
/// composition (AD-9), not the type.
pub struct ReadSideClaiming {
    pub store: Arc<dyn ReadSideClaimStore>,
    pub owner: OwnerId,
    pub clock: Arc<dyn Clock>,
    pub lease: chrono::Duration,
}

impl<E, H, RS, DS, OS, PR> ReadSideSession<E, H, RS, DS, OS, PR> {
    pub fn with_claiming(mut self, claiming: ReadSideClaiming) -> Self { … }

    pub async fn execute(&self) -> Result<Option<Offset>, ProjectionError> {
        let Some(c) = &self.claiming else { return self.run_batch(None).await };
        let Some(fence) = c.store
            .try_claim(&self.claim_id(), &c.owner, c.clock.now() + c.lease)
            .await
            .map_err(…)?
        else {
            return Ok(None); // refused: no fetch, no handler (AD-4)
        };
        let result = self.run_batch(Some(&fence)).await;
        let _ = c.store.release(&fence).await; // best-effort; see below
        result
    }
}
```

`run_batch` es el cuerpo actual de `execute()` textualmente, más una inserción entre
`handler.handle()` y el bucle de confirmación:

```rust
if let (Some(c), Some(fence)) = (&self.claiming, fence) {
    c.store.renew(fence, c.clock.now() + c.lease).await.map_err(|e| match e {
        ClaimError::StaleOwner => ProjectionError::transient(
            "claim lost before commit; this batch's offset and dedup writes were \
             withheld so a replaced owner cannot rewind the current owner's offset",
        ),
        other => ProjectionError::transient(format!("claim renew failed: {other}")),
    })?;
}
```

**Criterios**:

1. **El rechazo ocurre antes de `fetch`** (IS-4, primera línea de la Semántica Requerida): un
   trabajador rechazado no emite ningún `fetch` y no llega a ningún manejador, porque `try_claim`
   es la primera sentencia.
2. **La extracción a `run_batch` es lo que vuelve incondicional la liberación.** Rust no tiene
   `Drop` asíncrono, así que un guardia de ámbito no puede hacer `await`. Dividir el cuerpo es la
   construcción más pequeña que libera en la ruta de éxito, en ambas rutas de retorno temprano
   (`events.is_empty()`, `unique_events.is_empty()`) y en la ruta de error del manejador por igual.
3. **Un `release` fallido no es un fallo del lote** y se ignora deliberadamente: el trabajo ya se
   confirmó y el arriendo caduca por sí solo. Un `renew` fallido es lo opuesto — resguarda la
   escritura y debe propagarse.
4. **El `renew` es la compuerta de fencing, colocada lo más tarde posible.** Reverifica la tripleta
   completa en una sentencia y extiende a un arriendo completo y fresco en esa misma sentencia, así
   que la fase de confirmación corre dentro de un arriendo recién verificado.

**La ventana residual, nombrada en vez de disimulada.** Como `write_offset` y `mark_seen`
pertenecen a otros dos puertos que no portan fence (D-2) y no comparten transacción (D-11), la
compuerta de fencing y las escrituras que autoriza son **adyacentes, no atómicas**. Un trabajador
queda excluido en la compuerta — que es exactamente lo que pide la Semántica Requerida ("intenta
escribir … *como el propietario*" es rechazado, y el estado almacenado queda sin modificar) — pero
un trabajador cuyo `renew` *tuvo éxito* y cuya fase de confirmación luego sobreviva a un arriendo
entero recién concedido aún podría aterrizar un `write_offset` tardío. La cota es explícita y
configurable: la fase de confirmación (un `mark_seen` por evento más un upsert, sin código de
usuario) debe exceder la duración completa del arriendo. Es la misma clase de afirmación que hace
`reservation.rs:13-34` sobre su propio mecanismo — "la caducidad decide cuándo un intento está
*permitido*, y el fencing decide cuál resultado de la reserva es *autoritativo*. Ninguno hace imposibles dos
ejecuciones concurrentes". Cerrarlo del todo exige una transacción que abarque el reclamo, los
marcadores de dedup y el offset — que es el trabajo excluido por D-11, no el de este cambio.

**No se entrega renovación automática en segundo plano durante un `handler.handle()` largo.**
`renew` se entrega como capacidad, que es lo que pide la Semántica Requerida ("MUST be **able to**
extend the lease"); la duración del arriendo es configuración de despliegue y un manejador o
termina dentro de ella o es tomado legítimamente por otro. Es textualmente la decisión de
`operation/reservation.rs:19-27` — "una extensión deliberadamente diferida, no un descuido" —
adoptada aquí por la misma razón.

**La identidad del propietario es obligación del host, y debe documentarse en el knob.**
`ReadSideClaiming::owner` debe ser único por *instancia de proceso*. Dos procesos vivos que
compartan un `OwnerId` pueden satisfacer cada uno el `WHERE` de fence del otro, degradando la
exclusión de ejecución a solo caducidad de arriendo. El puerto no puede verificarlo; el rustdoc lo
declara y nombra la consecuencia.

### AD-7 — Planificador: un knob opcional de `ProjectionSpec`, sin cambio de firma del rasgo

**Decisión** — `crates/runtime/src/read_side/scheduler.rs`:

```rust
impl<F, H, S, D, O, R> ProjectionSpec<F, H, S, D, O, R> {
    /// Claims each `(tag, tenant)` stream before processing it. Absent by
    /// default, exactly as `reporter`/`interval`/`on_error` are.
    pub fn claims(mut self, claiming: ReadSideClaiming) -> Self { … }
}
```

`spawn` ya hace `let mut scheduler = self;` (`:301`), así que mueve `spec.claiming` a un campo
nuevo de `TagSchedulerImpl` antes del bucle; `start_projection` lee `self.claiming` y lo adjunta a
cada sesión que construye (`:90-100`).

**Criterios**: (a) `ProjectionSpec` ya existe precisamente para dar valores por defecto a knobs
opcionales (`:161-174`), así que esta es la forma establecida y no una nueva; (b) poner la
configuración en el *planificador* en vez de en el método del rasgo deja intacta la firma pública
de `TagScheduler::start_projection` — que ya tiene siete parámetros — y no rompe a ningún
implementador externo; (c) el alcance del reclamo es un lote, así que **no hay estado de reclamo
entre ticks, ni caché de fence en memoria, ni ciclo de vida nuevo que agregar al planificador** —
cada tick reclama, trabaja y libera. Dos sentencias extra por flujo por tick, nunca por evento
(R-3); (d) se mantienen OOS-5/D-12: `start_projection` sigue siendo el bucle `for` secuencial que
es hoy.

### AD-8 — Migración `016_create_projection_claims.sql`

**Decisión**:

```sql
-- Read-side processing claims: one row per (projection_id, tag, tenant),
-- naming the worker that currently holds single valid processing ownership
-- of that stream, until when, and under which fencing token.
--
-- IDENTITY — the primary key is byte-for-byte `projection_offsets`' identity
-- (013), which is the claim identity PROD-014C D-1 fixes. `tenant` is NOT NULL
-- for 013's reason: the read-side SPI's parameter is `tenant: &str`, never
-- `Option<&str>`, so there is no systemwide scope to model and no partial-index
-- pair is needed.
--
-- RELEASE IS AN EXPIRY, NOT A DELETE. A released claim is a row whose
-- `lease_until` has been set to the release instant, so `lease_until <= now`
-- is the single predicate meaning "claimable" for both released and lapsed
-- claims, and the fencing token stays strictly monotone for this identity's
-- whole life. Row count is therefore bounded by the number of streams ever
-- seen — the same bound `projection_offsets` has, and unlike
-- `projection_dedup` (014), which grows per event. No retention pass is
-- needed and none is shipped.
--
-- `claimed_at` is operational only; no decision reads it. Lease decisions are
-- made against the adapter's injected Clock, never the database's NOW()
-- (PROD-014C D-4).

CREATE TABLE IF NOT EXISTS projection_claims (
    projection_id VARCHAR(255) NOT NULL,
    tag           VARCHAR(255) NOT NULL,
    tenant        VARCHAR(255) NOT NULL,
    owner_id      VARCHAR(255) NOT NULL,
    fencing_token BIGINT       NOT NULL,
    lease_until   TIMESTAMPTZ  NOT NULL,
    claimed_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    CONSTRAINT projection_claims_identity
        PRIMARY KEY (projection_id, tag, tenant),
    CONSTRAINT projection_claims_fencing_token_positive
        CHECK (fencing_token > 0)
);
```

**Criterios**: (a) `016` es el siguiente número libre — la secuencia en `develop` termina en
`015_fix_aggregates_tenant_null_uniqueness.sql` (D-6); (b) la clave primaria **es** la identidad
única y **es** el único índice que necesita cada sentencia, siendo las cuatro búsquedas puntuales
sobre ella — PROD-014B AD-1 criterio 1, y sin clave sustituta `BIGSERIAL` porque un `tenant`
`NOT NULL` no necesita índices parciales; (c) **sin índice sobre `lease_until`**, deliberadamente:
nada escanea por arriendo, porque no hay ruta de purga ni cola; (d) `CHECK (fencing_token > 0)`
replica el de `010` y es la mitad de esquema del guardia de `token_from_storage`; (e) sin columna
`state` — a diferencia de `operation_reservations` no hay estado terminal `completed` que modelar,
ya que un reclamo liberado es un arriendo caducado; (f) `VARCHAR(255)` continúa la secuencia, y un
identificador demasiado largo se rechaza con `22001` → `Fatal`, nunca se trunca; (g) el registro es
una const `include_str!` y una entrada añadida en `migrations()`, y la prueba bidireccional de
registro ya existente en `migrations.rs` falla si el archivo se entrega sin registrar — no se
escribe ninguna prueba de migración nueva (PROD-014B AD-2).

### AD-9 — La compuerta de Producción reutiliza `require_durably_configured` textualmente

**Decisión**: sin predicado nuevo. `crates/service-sdk/src/runtime/builder.rs` gana un slot y un
validador hermano, invocado desde `validate_persistence_profile` después de los dos existentes:

```rust
read_side_claims: Option<Arc<dyn ReadSideClaimStore + Send + Sync>>,

/// Under `Profile::Production`, a composition that registers read-side
/// progress must also register a durable claim store (PROD-014C IS-6).
///
/// The early return is INSIDE this function, never before the call: an
/// early return placed in `validate_persistence_profile` would skip this
/// check for every composition that registers no read-side progress AND
/// every one that does — PROD-014A EC-1's exact defect.
fn validate_read_side_claim_profile(&self) -> Result<(), RuntimeError> {
    // Registration of a progress pair is the composition-visible signal that
    // this application processes a read side at all. A command-only service
    // is never forced to register a claim store it would never use
    // (PROD-014A IS-5, and `validate_effect_store_profile`'s own shape).
    if self.read_side_progress.is_empty() {
        return Ok(());
    }
    persistent_entity::profile::require_durably_configured(
        self.profile,
        self.read_side_claims.as_ref().is_some_and(|c| c.is_durable()),
        "durable read-side claim store (ReadSideClaimStore)",
        "AppBuilder::read_side_claims(store) (or \
         RuntimeBuilder::with_read_side_claim_store(..)), passing a store whose \
         is_durable() returns true",
    )?;
    Ok(())
}
```

**Criterios**: (a) D-5 pregunta si la compuerta reutiliza `require_durably_configured` textualmente
— lo hace, sin cambios, porque todo el trabajo del predicado es `Production && !durably_configured
⇒ rechazar` y esa es exactamente esta regla; (b) el argumento `is_some_and(|c| c.is_durable())` es
el mismo modismo que ya usa `validate_effect_store_profile` para el effect store
(`builder.rs:863-865`), y es la composición de *presencia* y *durabilidad* que `profile.rs:41-50`
advierte que nunca debe colapsar en `.is_some()`; (c) **un slot global, no un mapa por proyección**:
`projection_id` es parte de la identidad del reclamo, así que un solo store sirve a todas las
proyecciones — a diferencia de un par de progreso, que es inherentemente por proyección;
(d) el registro replica la división de `effect_store` — gana la última escritura en `RuntimeBuilder`,
guardia de duplicados que falla cerrado en `AppBuilder` (`CompositionError::DuplicateReadSideClaimStore`),
que es la división establecida por PROD-014A y evita la segunda verificación paralela.

**Postura posterior al cambio (IS-6)**: el lado de lectura multi-réplica pasa a estar **soportado
bajo una restricción operativa explícita** — un claim store durable registrado, `OwnerId` únicos
por proceso, y efectos del manejador todavía al-menos-una-vez (D-8, OOS-2).

### AD-10 — Pruebas: unitarias para forma y compuerta, PostgreSQL real para toda afirmación de contención

**Decisión** según D-7: una afirmación de atomicidad es una afirmación sobre lo que hace la base de
datos bajo contención real, así que ninguna prueba unitaria la simula. TDD estricto — la suite de
contención se escribe en ROJO contra tipos que todavía no compilan.

| Nivel | Ubicación | Qué demuestra |
|---|---|---|
| Unitaria | `persistence-api/src/read_side/claim.rs` `#[cfg(test)]` | AD-3: `Arc<T>` reenvía `is_durable()` (la trampa PROD-014A EC-2) y los tres métodos; identidad de `ClaimId`/`ClaimFence` |
| Unitaria | `domain/src/read_side/session.rs` `#[cfg(test)]`, dobles guionados, **sin pool** | Orden de AD-6: un `try_claim` rechazado ⇒ nunca se llama `fetch`, nunca se invoca el manejador, `Ok(None)`; un `renew` que devuelve `StaleOwner` ⇒ **ningún** `mark_seen` y **ningún** `write_offset`, el error se propaga; `release` se llama en la ruta de éxito, en ambas rutas de retorno temprano vacías y en la ruta de error del manejador |
| Unitaria | `service-sdk/src/runtime/builder.rs` `#[cfg(test)]` | Matriz de AD-9: Production + progreso registrado + sin claim store ⇒ rechaza nombrando capacidad y arreglo; + claim store volátil ⇒ rechaza; + durable ⇒ ok; Production + **cero** progreso registrado + sin claim store ⇒ ok (la prueba con forma de EC-1 para el retorno temprano); Dev + nada ⇒ ok; `build()` y `try_build()` concuerdan |
| Unitaria | `persistence/src/postgres/migrations.rs` (pruebas existentes, sin código nuevo) | `016` está registrada y ordenada |
| Integración (PG real) | `integration-tests/tests/infrastructure/read_side_claiming_postgres.rs` | SC-1, SC-2, SC-3, SC-5, SC-7 — abajo |

Arnés, copiado de `takeover_fencing_postgres.rs` y `concurrent_replicas_postgres.rs`:
`isolated_database()` por prueba; **pools `PgPoolOptions` separados por contendiente** para que no
compartan conexión; `SettableClock` movido a mano para alcanzar la caducidad sin dormir;
`tokio::sync::Barrier` que libera ambos intentos juntos; observadores `AtomicUsize` que registran
sin participar; toda espera acotada por una aserción `WAIT_LIMIT` en vez de un timeout silencioso;
el estado final se lee de vuelta con `sqlx::query_as` crudo, **nunca a través del puerto bajo
prueba**; `db.close().await` al final.

- **SC-1 — exclusión de ejecución.** Dos trabajadores, dos pools, dos `OwnerId`, liberados juntos
  sobre el mismo `(projection_id, tag, tenant)`: exactamente uno obtiene `Some(fence)`, el otro
  `None`; los contadores de `fetch` y de manejador del rechazado son ambos **0**. *Caso de
  control*, según el propio de `concurrent_replicas_postgres.rs:581-592`: los mismos dos
  trabajadores sobre dos tenants distintos obtienen ambos un fence y ambos corren — sin él, un
  arnés que rechazara lo que llegara segundo satisfaría cada aserción anterior sin demostrar nada.
- **SC-2 — toma de control sin acción del operador.** A reclama y nunca libera (su sesión se
  descarta a mitad de lote — la muerte modelada); el reloj se avanza más allá de `lease_until`; el
  `try_claim` de B devuelve `Some`, con `fencing_token` **estrictamente mayor**, y el `owner_id` de
  la fila es el de B.
- **SC-3 — un propietario obsoleto no puede escribir como propietario.** Tras la toma de control de
  B: `renew(a_fence)` y `release(a_fence)` son ambos `Err(StaleOwner)` y la fila sigue mostrando el
  propietario y el token de B, sin cambios. Luego, a nivel de sesión: el lote de A se conduce por
  su fase de confirmación con el fence obsoleto, y `projection_offsets` se lee de vuelta con SQL
  crudo y **sigue conteniendo el valor de B** — el retroceso no ocurrió. Además la sonda de
  aislamiento de token de `takeover_fencing_postgres.rs:182-213`: **el propietario B con el token
  obsoleto de A** también debe ser rechazado, para que el rechazo no pueda atribuirse solo a
  `owner_id`.
- **SC-5 — orden sin cambios.** Un trabajador sostiene el reclamo a lo largo de un lote de al menos
  tres eventos; se verifica que el slice recibido por el manejador es estrictamente ascendente por
  `event_version`.
- **Verificaciones por mutación, medidas en vez de asumidas** (el hábito que establecen
  `reservation.rs:286-298` y `takeover_fencing_postgres.rs:182-196`): borrar
  `AND projection_claims.lease_until <= $6` del `WHERE` del `ON CONFLICT` de `try_claim` debe hacer
  fallar SC-1 con ambos trabajadores reclamando; borrar `AND fencing_token = $6` del `WHERE` de
  fence compartido debe hacer fallar la sonda de token de SC-3. Ambas quedan registradas en el doc
  de módulo de la suite.

Dos propiedades son propiedades del diff, verificadas leyendo el cambio y no por una prueba:
**SC-6** (ningún artefacto entregado dice exactamente-una-vez — la compuerta de grep que nombra
R-1) y la lista de archivos sin tocar del Mapa de Componentes (`read_side_offset.rs`,
`read_side_dedup.rs`, `offset.rs`, `dedup.rs`).

---

## Puntos de Integración

| Frontera | Dirección | Mecanismo | Verificado en |
|---|---|---|---|
| `persistence-api` → `ego-domain` | arriba | bloque de re-export de módulos existente | `domain/src/read_side/mod.rs:21-24` |
| puerto → borrado a `Arc<dyn …>` | afuera | nueva impl general (AD-3) | replica `offset.rs:91-119` |
| `is_durable()` → `Profile::Production` | adentro | `require_durably_configured`, sin cambios | `profile.rs:51-63`; AD-9 |
| `AppBuilder` → `RuntimeBuilder` | abajo | delegación fina + guardia de duplicados | replica `app/mod.rs:590-624` |
| `ProjectionSpec` → sesión | afuera | un knob opcional, movido a través de `spawn` | `scheduler.rs:288-301`; AD-7 |
| esquema → runtime | adentro | ejecutor existente `include_str!` + `raw_sql` | `migrations.rs`; AD-8 |
| adaptador → `Clock` | adentro | `Arc<dyn Clock>` por constructor | `reservation.rs:85-87`; D-4 |
| puerto de reclamo → `OffsetStore`/`DedupStore` | **ninguna** | no se agrega ruta, no existe ninguna | D-2, D-11 |

## Matriz de Amenazas

N/A — no hay frontera de enrutamiento, comando de shell, subproceso, automatización de VCS/PR,
clasificación de archivos ejecutables ni integración de procesos. Este cambio agrega un SPI, un
adaptador SQL, un archivo DDL, una compuerta de composición y una suite de pruebas; no se invoca
ningún proceso externo y no se ejecuta ni clasifica ningún archivo.

La superficie aplicable son las Reglas 1 y 2 de `ego-rs-security`, cerradas por construcción: todo
valor es un `$N` ligado y nada — identificador o valor — se interpola en el texto SQL (AD-5).
`projection_claims` **sí** es dato con alcance de tenant y liga `tenant` en cada sentencia, así que
la excepción de `projection_dedup` de PROD-014B AD-7 no aplica aquí y no se reutiliza.

## Migración / Despliegue

Un `CREATE TABLE IF NOT EXISTS` aditivo, no referenciado por nada más y que no referencia nada
más. Ninguna tabla, columna, índice o consulta existente cambia. El orden de despliegue es el
existente: `migrations::run` ya precede a toda construcción de stores.

El despliegue progresivo es seguro en ambas direcciones y vale la pena declararlo, porque es el
despliegue para el que existe este cambio. Las réplicas nuevas reclaman; las viejas no. Durante el
solapamiento una réplica vieja aún puede procesar un flujo que una nueva posee — que es el
comportamiento de hoy, no una regresión introducida por el despliegue parcial — y la garantía se
vuelve efectiva una vez que la última réplica vieja desaparece. Bajo `Profile::Production` la flota
no puede arrancar *sin* un claim store una vez que esto se entrega, así que el solapamiento queda
acotado por el propio despliegue.

La reversión es la de la propuesta: eliminar el puerto, el adaptador, el re-export y la compuerta;
descartar `016` y su entrada de registro; quitar el knob de `ProjectionSpec` y la llamada a
`with_claiming`; restaurar la restricción de adopción de escritor único en specs y documentación.
La tabla puede descartarse o dejarse — nada más la referencia, y descartarla degrada el
comportamiento al de PROD-014B, no a corrupción.

## Trazabilidad

| Ítem de la propuesta | Resuelto por |
|---|---|
| D-1, IS-1 | AD-2 (`ClaimId`), AD-8 (PK idéntica a la de `013`) |
| D-2, IS-2 | AD-1, AD-3 — puerto nuevo, reenvío `Arc<T>`, `OffsetStore`/`DedupStore` sin tocar |
| D-3, IS-3, (D) | AD-2, AD-5, AD-6 — token reutilizado, acuñado en SQL, verificado en cada `WHERE` |
| D-4, R-5 | AD-5 (`Clock` inyectado del adaptador), AD-6 (`ReadSideClaiming::clock`) |
| D-5, IS-6, SC-4 | AD-9 — `require_durably_configured` textual, retorno temprano dentro de la función |
| D-6, IS-5 | AD-8 — `016`, una fila por flujo, sin retención |
| D-7, IS-7, SC-7 | AD-10 — `isolated_database()`, pools separados, sin simulación a nivel unitario |
| D-8, OOS-2, R-1, SC-6 | Nota de redacción al inicio; declaración de ventana residual de AD-6; SC-6 es propiedad del diff |
| D-9 | AD-5 — una fila, una sentencia; sin consenso, sin elección de líder, sin broker |
| D-10, OOS-3 | Sin tocar: no se agrega reintento/backoff |
| D-11, OOS-4 | AD-6 — nombrado como causa de la ventana residual, no cerrado |
| D-12, OOS-5 | AD-7(d) — `start_projection` sigue siendo secuencial |
| IS-4, SC-1 | AD-6 — `try_claim` es la primera sentencia; el rechazo precede a `fetch` |
| SC-2 | Ruta de toma de control de AD-5; SC-2 de AD-10 |
| SC-3 | `WHERE` de fence de AD-5; compuerta previa a confirmación de AD-6; SC-3 de AD-10 |
| SC-5 | AD-7(c) el reclamo es por flujo; SC-5 de AD-10 |
| R-2 | AD-6 — capacidad `renew`, arriendo como configuración, el fencing rechaza al escritor desalojado |
| R-3 | AD-7(c) — por flujo y por lote, nunca por evento |
| R-4 | `sdd-tasks` es dueño del pronóstico de 400 líneas; no se anticipa aquí |
| Pregunta abierta del Enfoque | La comparación A–E: se elige (C), tres métodos y no cuatro |

## Preguntas Abiertas

- [ ] AD-6 — la compuerta de fencing y las escrituras que autoriza son adyacentes, no atómicas,
      porque D-2 y D-11 son ambas vinculantes. Confirmar que la cota declarada (una fase de
      confirmación que sobreviva a un arriendo completo recién concedido) es aceptable, en vez de
      promover el trabajo de transacción compartida que D-11 excluyó.
- [ ] AD-6 — la unicidad de `OwnerId` por proceso es una obligación del host que el puerto no
      puede verificar. Confirmar que documentarla en el knob es suficiente, en vez de que el
      framework derive por sí mismo un identificador de propietario.
- [ ] AD-9 — la compuerta se activa por "hay un par de progreso registrado". Una proyección lanzada
      directamente por `ProjectionSpec`/`TagSchedulerImpl` sin pasar por la raíz de composición
      queda sin gobernar, exactamente como ya establece PROD-014A OOS-7. Confirmar que esa frontera
      queda intencionalmente sin cambios aquí.
