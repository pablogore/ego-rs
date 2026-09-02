# Diseño: PROD-014B — Almacenes de Lado de Lectura Durables en PostgreSQL

> Compañero de revisión en español. Fuente de verdad canónica: `design.md` (identificadores 1:1).
>
> **Insumos**: `proposal.md` (D-1 … D-8, IS-1 … IS-9, OOS-1 … OOS-8, G-1 … G-4, L-1 … L-5,
> R-1 … R-6, F-1 … F-3, SC-1 … SC-12) y `explore.md` (§7 Adenda de Contrato y Concurrencia,
> Q1 … Q5). Este documento decide el **cómo**: esquema, SQL, enfoque de concurrencia, manejo
> de errores, ubicación y ubicación de pruebas. Los requisitos observables pertenecen a
> `spec.md` y no se repiten aquí.
>
> **Línea base leída**: `develop` @ `a445e5b`. Cada `archivo:línea` citado abajo fue leído
> sobre esta línea base, no recordado.

## Enfoque Técnico

Dos tablas, dos adaptadores, dos migraciones, sobre el camino dorado que `event_store.rs` /
`snapshot.rs` / `reservation.rs` ya establecieron: un archivo por almacén bajo
`crates/persistence/src/postgres/`, `PgPool` por inyección en el constructor, `is_durable()`
fijo en `true`, reexportado desde `postgres/mod.rs`, y el esquema entregado como los
siguientes números de la secuencia plana `include_str!` + `sqlx::raw_sql`.

Ambas escrituras son seguras ante conflictos por construcción, no por coordinación.
`write_offset` es un único upsert; `mark_seen` es un único
`INSERT … ON CONFLICT … DO NOTHING`. Ninguna toma un candado, un arriendo ni un testigo de
esgrima, y ninguna necesita un viaje de lectura-modificación-escritura.

Lo que eso compra se enuncia con precisión en **AD-6**, y es la frase portante de este
diseño: la garantía entregada es **un-solo-escritor-por-`(projection_id, tag, tenant)`**
(L-3 de la propuesta). Las escrituras seguras ante conflictos hacen converger la
*contabilidad*. No cierran la ventana de comprobar-luego-actuar entre `seen()` y
`mark_seen()`, porque el manejador ya se ejecutó dentro de ella. Cerrarla es
**PROD-014C — Atomic Read-Side Event Claiming** (F-1), y este diseño no lo intenta.

No se toca código de SPI, compuerta, registro ni planificador (OOS-1, OOS-2, D-6).

---

## Correcciones de Evidencia

Ambas se hallaron leyendo el código al que apuntan los insumos. Cada una cambia lo que la
implementación debe hacer.

### EC-1 — `explore.md` §2 recomienda el `UPDATE` condicional de `reservation.rs` para la escritura de offset; §7 Q4 lo deja sin efecto

`explore.md:83-87` lee el camino dorado como "`UPDATE … WHERE <identidad + versión
esperada>` condicional para actualizaciones disputadas — el patrón aplicable para un
adaptador durable de offset/dedup, **no un upsert plano**", y §5 ítem 3 lo repite. La
adenda posterior revierte esto con evidencia: `write_offset` (`offset.rs:77-83`) no tiene
parámetro de valor previo esperado ni lenguaje de ordenamiento, y su único llamador
(`session.rs:91-176`) nunca inspecciona si su escritura ganó (`explore.md` Q4; D-3, L-5 de
la propuesta).

Un adaptador que llevara el `UPDATE` condicional de `reservation.rs` rechazaría escrituras
que el trait considera válidas — un almacén más estricto que el contrato que implementa,
que falla para llamadores que el SPI admite. **AD-3 implementa el upsert plano.** Los dos
textos se contradicen; esto no es un descuido que se esté rodeando en silencio.

### EC-2 — El pool de `main.rs` se *mueve* dentro de `EntityEventStores::open`, así que IS-6 necesita un clon tomado antes de esa línea

`examples/reference-app/src/main.rs:73-78` conecta el pool y luego lo pasa **por valor**:
`EntityEventStores::open(pool)`. `PgPool` es `Clone` (un `Arc` internamente), así que el
arreglo es un `pool.clone()` — pero debe tomarse *antes* de la línea 78, no después, y la
descripción de una línea de IS-6 ("recablear `None` al par Postgres real") no deja ver que
para entonces el binding ya no existe. Resuelto en **AD-10**, que además fija el orden
respecto de `migrations::run(&pool)` en la línea 77.

---

## Mapa de Componentes

```
crates/persistence/src/postgres/
├── migrations/013_create_projection_offsets.sql   NUEVO  AD-1, AD-2
├── migrations/014_create_projection_dedup.sql     NUEVO  AD-1, AD-2
├── migrations.rs                     MOD  dos const + dos entradas de registro (AD-2)
├── read_side_offset.rs               NUEVO  PostgreSQLOffsetStore  (AD-3, AD-4)
├── read_side_dedup.rs                NUEVO  PostgreSQLDedupStore   (AD-5)
└── mod.rs                            MOD  dos `pub use` + pub(crate) fn is_fatal (AD-8, AD-9)
                                                ↑ usado por
examples/reference-app/
├── src/read_side/mod.rs              MOD  ReadSideProgressStores::postgres(pool)  (AD-10)
└── src/main.rs                       MOD  pool.clone() antes de open; None → Some  (AD-10, EC-2)

integration-tests/tests/infrastructure/
└── read_side_progress_postgres.rs    NUEVO  toda la suite de conformidad  (AD-12)

crates/domain/src/read_side/{offset,dedup,session,runner}.rs   INTACTO  (OOS-1)
crates/service-sdk/src/{runtime/builder.rs, app/mod.rs}        INTACTO  (OOS-1)
crates/runtime/src/read_side/scheduler.rs                      INTACTO  (OOS-2)
crates/effect-store/                                           INTACTO  (secuencia 001/002 propia)
```

## Flujo de Datos

```
ReadSideSession::execute (session.rs, SIN CAMBIOS)        PostgreSQL
─────────────────────────────────────────────────         ──────────
 Fase 2  dedup.seen(pid, tag, event_id) ───────────▶  SELECT 1 FROM projection_dedup
             │  false                                  WHERE pid=$1 AND tag=$2 AND event_id=$3
             ▼                                                    │
 Fase 3  handler.handle(event)   ◀── ⚠ LA VENTANA ────────────────┘
             │      dos escritores pueden estar ambos aquí, ambos habiendo
             │      leído false, antes de que cualquiera llegue a la Fase 4
             ▼                                        INSERT INTO projection_dedup …
 Fase 4  dedup.mark_seen(...) ─────────────────────▶  ON CONFLICT (pid,tag,event_id) DO NOTHING
             │                                        → converge a UNA fila, sin error (G-2)
             ▼                                        INSERT INTO projection_offsets …
         offset.write_offset(..., Sequence(n)) ────▶  ON CONFLICT (pid,tag,tenant)
                                                      DO UPDATE SET offset_value = EXCLUDED…
                                                      → gana la última escritura (L-5)
```

La ventana marcada ⚠ es L-2/L-3, no alterada por este diseño y propiedad de **PROD-014C —
Atomic Read-Side Event Claiming** (F-1). Ver AD-6.

---

## Decisiones de Arquitectura

### AD-1 — Esquema: dos tablas, claves primarias compuestas, `tenant` `NOT NULL` solo en offsets

**Decisión** — `013_create_projection_offsets.sql`:

```sql
CREATE TABLE IF NOT EXISTS projection_offsets (
    projection_id VARCHAR(255) NOT NULL,
    tag           VARCHAR(255) NOT NULL,
    tenant        VARCHAR(255) NOT NULL,
    offset_value  BIGINT       NOT NULL,
    updated_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    CONSTRAINT projection_offsets_identity
        PRIMARY KEY (projection_id, tag, tenant)
);
```

`014_create_projection_dedup.sql`:

```sql
CREATE TABLE IF NOT EXISTS projection_dedup (
    projection_id VARCHAR(255) NOT NULL,
    tag           VARCHAR(255) NOT NULL,
    event_id      VARCHAR(255) NOT NULL,
    created_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    CONSTRAINT projection_dedup_identity
        PRIMARY KEY (projection_id, tag, event_id)
);
```

**Criterios**:

1. **La clave primaria *es* la identidad UNIQUE que pide IS-2**, y además es el índice que
   lee `seen()`. Un `BIGSERIAL id` + índice único aparte (la forma de `010`/`011`) existe
   allí únicamente porque un `tenant_id` anulable obligaba a dos índices únicos
   *parciales*, que una restricción de tabla no puede expresar. Esa razón está
   estructuralmente ausente aquí (D-2), así que la clave sustituta sería una columna que
   nadie lee y un segundo índice que nadie necesita.
2. **`tenant` es `NOT NULL`, y el patrón `tenant_id IS NOT DISTINCT FROM $N` de
   `reservation.rs:182` / `snapshot.rs:67,77` deliberadamente no se reutiliza** (D-2, Q3).
   El parámetro del SPI de lado de lectura es `tenant: &str`, nunca `Option<&str>`; el
   concepto de inquilino "systemwide" del framework
   (`crates/domain/src/persistence/tenant.rs:29-35`) es un tipo del lado de escritura sin
   contraparte en el lado de lectura. Una columna anulable modelaría un estado que el SPI
   no puede producir, y arrastraría dos índices parciales para hacerlo.
3. **`offset_value`, nunca `offset`.** `OFFSET` es palabra reservada en PostgreSQL; una
   columna llamada `offset` es inusable sin comillas, y entrecomillar un identificador
   durante toda la vida de la tabla para ahorrar cuatro caracteres es una trampa, no una
   convención.
4. **`BIGINT` es exactamente `Offset::Sequence(i64)`** (`offset.rs:12-16`) — un mapeo total
   en ambas direcciones, así que no se requiere una conversión verificada al estilo
   `token_for_storage` (AD-4).
5. **`VARCHAR(255)` sigue la secuencia existente** (`001`, `010`, `011`) en vez de inventar
   `TEXT`. Su techo, enunciado en lugar de descubierto: un identificador de más de 255
   caracteres es **rechazado** por la columna (SQLSTATE `22001`), no truncado, y AD-8 lo
   clasifica como `Fatal`. Ningún sitio de llamada produce uno — `projection_id` es un
   `const`, `tag` lo compone el host, `tenant` es un `TenantId` validado.
6. **`created_at` / `updated_at` son operacionales, no funcionales.** Ninguna consulta los
   lee. `created_at` en `projection_dedup` es lo que una futura pasada de retención F-2
   recorrería; este cambio no entrega índice para eso, porque indexar para un barrido que
   nadie ejecuta es diseñar F-2 por anticipado (AD-11).

**Rechazado — una tabla fusionada.** Las dos identidades difieren en una columna que no es
opcional en ninguno de los dos lados: una fila de offset no tiene `event_id` y una fila de
dedup no tiene `tenant` (D-1). Una tabla fusionada necesita ambas anulables más un
discriminador, lo que convierte dos claves primarias en dos índices únicos parciales y un
`CHECK` — estrictamente más esquema para expresar estrictamente menos.

### AD-2 — Dos archivos de migración en `013`/`014`, registrados en la secuencia plana existente

**Decisión**: dos constantes `include_str!` y dos entradas añadidas a `migrations()`
(`migrations.rs:60-90`), en orden ascendente, ejecutadas por el bucle `sqlx::raw_sql` sin
cambios (`migrations.rs:43-57`).

**Criterios**: (a) cada archivo existente de la secuencia crea o altera exactamente una
cosa — `010` y `011` son dos tablas en dos archivos para la misma capacidad, que es
precisamente este caso; (b) la secuencia independiente `001/002` de `crates/effect-store`
es una propiedad de un crate separado (su AD-10), no una regla que se traslade a
`ego-persistence` (D-5); (c) **las pruebas existentes en `migrations.rs` ya cubren los
archivos nuevos sin costo alguno** —
`every_migration_file_is_registered_and_every_registration_has_a_file` falla si se agrega
un `.sql` sin registrarlo (el defecto exacto que una vez dejó tres migraciones inertes), y
`registration_order_ascends_by_numeric_prefix` falla si `013`/`014` quedan desordenados. No
se escribe ninguna prueba de migración nueva; R-4 lo atrapa maquinaria que ya existe.

### AD-3 — `write_offset` es un upsert, gana la última escritura (resuelve EC-1)

**Decisión**:

```rust
sqlx::query(
    r#"INSERT INTO projection_offsets (projection_id, tag, tenant, offset_value)
       VALUES ($1, $2, $3, $4)
       ON CONFLICT (projection_id, tag, tenant)
       DO UPDATE SET offset_value = EXCLUDED.offset_value, updated_at = NOW()"#,
)
.bind(projection_id)
.bind(tag.value())
.bind(tenant)
.bind(offset.as_sequence().expect("Offset has exactly one variant"))
.execute(&self.pool)
.await
.map_err(offset_error)?;
```

**Criterios**: (a) EC-1 — el SPI expresa sobrescritura, y el adaptador implementa el
contrato que le entregaron, no uno más estricto; (b) una sola sentencia, así que no hay
ventana de comprobar-luego-escribir creada por el propio adaptador — dos escritores
concurrentes tienen éxito ambos y gana el commit posterior, lo cual es `Ok(())` bajo este
SPI (L-5); (c) `tag.value()` (`event_tag.rs:29-31`) es el `$2` ligado, nunca la forma
`Display` y nunca interpolado.

**El `.expect(...)` sobre `as_sequence()` es correcto aquí**, no un pánico latente:
`Offset` es un enum de una sola variante (`offset.rs:12-16`) cuya restricción FR-014 es
que siga siéndolo. Si alguna vez se agrega una variante, este es exactamente el punto donde
la falla debe aflorar, en lugar de escribir en silencio una secuencia fabricada.

### AD-4 — `read_offset` es una búsqueda escalar puntual; ausente significa `Ok(None)`

**Decisión**:

```rust
let stored: Option<i64> = sqlx::query_scalar(
    r#"SELECT offset_value FROM projection_offsets
       WHERE projection_id = $1 AND tag = $2 AND tenant = $3"#,
)
.bind(projection_id).bind(tag.value()).bind(tenant)
.fetch_optional(&self.pool).await.map_err(offset_error)?;

Ok(stored.map(Offset::Sequence))
```

**Criterios**: (a) `fetch_optional`, así que "nunca escrito" es `Ok(None)` y no un error —
el caso de reanudar desde cero es normal, no excepcional; (b) las tres columnas de
identidad van ligadas como `$N`, incluida `tenant`, que es SC-7 y la Regla 2 de
`ego-rs-security` satisfechas por la forma de la consulta y no por una promesa de revisión;
(c) la clave primaria hace que a lo sumo una fila coincida, así que `fetch_optional` no
puede alcanzar la ruta de error de múltiples filas; (d) `i64` → `Offset` no necesita
guarda (criterio 4 de AD-1).

### AD-5 — `mark_seen` es un `INSERT … ON CONFLICT (…) DO NOTHING` con objetivo explícito

**Decisión**:

```rust
// mark_seen
sqlx::query(
    r#"INSERT INTO projection_dedup (projection_id, tag, event_id)
       VALUES ($1, $2, $3)
       ON CONFLICT (projection_id, tag, event_id) DO NOTHING"#,
)
.bind(projection_id).bind(tag.value()).bind(event_id)
.execute(&self.pool).await.map_err(dedup_error)?;
Ok(())

// seen
let hit: Option<i32> = sqlx::query_scalar(
    r#"SELECT 1 FROM projection_dedup
       WHERE projection_id = $1 AND tag = $2 AND event_id = $3"#,
)
.bind(projection_id).bind(tag.value()).bind(event_id)
.fetch_optional(&self.pool).await.map_err(dedup_error)?;
Ok(hit.is_some())
```

**Criterios**: (a) una sentencia, sin inspeccionar `rows_affected` — el SPI devuelve
`Result<(), _>`, así que "insertado" y "ya estaba" son el mismo éxito, y leer el conteo
solo tentaría a una distinción visible al llamador que el trait no puede transportar;
(b) **un objetivo de conflicto explícito, a diferencia del `ON CONFLICT DO NOTHING` desnudo
de `reservation.rs:213-219`** — allí esa forma es forzada por índices parciales, que no
pueden nombrarse como objetivo; aquí la identidad es una clave primaria simple, y
nombrarla hace que la violación de alguna *otra* restricción futura aflore como error en
vez de ser tragada en silencio; (c) `seen()` es una búsqueda puntual por clave primaria,
así que sin `LIMIT` y sin `COUNT(*)`.

**Estudiado y deliberadamente no copiado: `EffectDedupStore::reserve`**
(`crates/effect-store/src/postgres/mod.rs:699-756`). Es el mismo insert atómico único, pero
*devuelve el resultado* — `rows_affected() == 1` significa que el llamador ganó la reserva,
y corre **antes** de cualquier efecto colateral. Esa es la forma que F-1 necesita y la
forma que este SPI no puede expresar: `mark_seen` devuelve `Result<()>`, se invoca solo
después de `handler.handle()` (`session.rs:135`), y no tiene vocabulario para "perdiste".
Reproducir aquí la sentencia de `reserve` sin su tipo de retorno ni su posición de llamada
aparentaría exclusión sin proveer ninguna.

### AD-6 — La garantía entregada es un-solo-escritor-por-`(projection_id, tag, tenant)`; la clave primaria es almacenamiento idempotente, no exclusión

**Decisión**: este par se diseña, documenta y prueba como **procesamiento al-menos-una-vez
con contabilidad de deduplicación de mejor esfuerzo bajo una suposición de un solo escritor
no aplicada por nadie** (L-1/L-2/L-3 de la propuesta). Nada en los adaptadores, su rustdoc,
el README de persistencia ni la documentación de configuración puede describirlos como
exactamente-una-vez, seguros ante concurrencia, o seguros para un escritor de proyección
multi-réplica (SC-8, SC-12, IS-8).

**La distinción que este diseño está obligado a hacer explícita**:

| | Lo que hace la clave primaria de `projection_dedup` | Lo que no hace |
|---|---|---|
| Efecto | Dos `mark_seen` concurrentes con la misma identidad convergen a **una fila**, sin que ningún error de violación de unicidad aflore a ninguno de los dos llamadores (G-2) | Impedir que **dos manejadores ya se hayan ejecutado** |
| Por qué | `ON CONFLICT DO NOTHING` resuelve la carrera de escritura dentro de una sentencia | La carrera está aguas arriba: `seen()` (`session.rs:116-128`) y `mark_seen()` (`:142-149`) son métodos separados del SPI con `handler.handle()` (`:135`) en medio. Ambos escritores leen `false` y ejecutan **antes** de que cualquiera marque (`explore.md` Q5) |
| Alcance | Idempotencia de almacenamiento | Nada sobre exclusión de ejecución |

Una restricción sobre una tabla es un predicado sobre filas. No puede des-ejecutar
retroactivamente un efecto que ya ocurrió, y ningún adaptador de PostgreSQL puede cerrar
esta ventana desde dentro de `mark_seen` — es un hueco a nivel de SPI (veredicto de Q5,
D-7).

El un-solo-escritor-por-tag se cumple **dentro de un proceso hoy** porque
`TagSchedulerImpl::start_projection` (`scheduler.rs:66-108`) espera la sesión de cada tag
secuencialmente. Entre réplicas nada lo aplica: el código de lado de lectura no contiene
elección de líder, ni candado, ni arriendo, ni testigo de esgrima. Este cambio ni detecta
ni rechaza una segunda réplica (OOS-2).

**Dónde engancharía una futura reclamación atómica, solo nombrado**: la costura son las
Fases 2/3 de `ReadSideSession::execute` — una reclamación debe *obtenerse* antes de que
corra `handler.handle()` y debe devolver si el llamador ganó, lo que implica un método
nuevo del SPI (o aplicación de un solo escritor a nivel de orquestación), no una sentencia
SQL distinta detrás de la existente. Eso es **PROD-014C — Atomic Read-Side Event Claiming**
(F-1). Aquí se nombra y en ningún punto de este documento se diseña.

### AD-7 — `projection_dedup` no lleva columna `tenant`, y eso no es un defecto de aislamiento de inquilinos

**Decisión**: la identidad de dedup es exactamente `(projection_id, tag, event_id)` (D-1,
Q1). El inquilino está ausente de la tabla, de toda consulta de dedup y del índice.

**Criterios**: `seen`/`mark_seen` del SPI (`dedup.rs:37-51`) no toman inquilino, y la
documentación del trait enuncia el alcance de frente ("Deduplication scope:
(projection_id, tag, event_id)"). Agregar una columna que ningún método del SPI puede
poblar haría del inquilino de cada fila una invención.

**Lo que hay que decirle a quien revisa seguridad en vez de dejarlo inferir** (la Regla 2
de `ego-rs-security` pide que toda tabla con datos por inquilino ligue `tenant_id`): esta
tabla **no** contiene datos por inquilino. No almacena ningún valor propiedad de un
inquilino — solo la presencia de un identificador de evento — y `projection_offsets`, que
sí es por inquilino, liga `tenant` en cada sentencia (AD-3, AD-4). La consecuencia es real
y requerida por SC-4: el mismo `event_id` bajo el mismo `(projection_id, tag)` se considera
ya visto sin importar el inquilino. En la composición de referencia eso es inerte, porque
`tag` es a su vez derivado del inquilino (`tenant_tag`, `read_side/mod.rs:199`), de modo
que dos inquilinos nunca comparten un tag. Un host que elija un tag independiente del
inquilino **sí** compartirá filas de dedup entre inquilinos — una propiedad de la identidad
del SPI, adoptada aquí a sabiendas, no introducida por este adaptador.

### AD-8 — Un único predicado compartido `is_fatal` basado en SQLSTATE; `Transient` es el valor por defecto

**Decisión**: `crates/persistence/src/postgres/mod.rs` gana una función pura `pub(crate)`, y
cada adaptador la mapea a su propio tipo de error.

```rust
/// Whether a storage failure will fail the same way on every retry.
///
/// `Transient` is the default because a retryable failure misreported as `Fatal`
/// stops a projection that would have recovered on its own. The four codes below
/// are the ones a retry cannot help: the migration did not run, the schema drifted,
/// a value does not fit its column, or a row cannot be decoded into the type this
/// crate wrote.
pub(crate) fn is_fatal(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db) => matches!(
            db.code().as_deref(),
            Some("42P01") // undefined_table — migration 013/014 not applied
                | Some("42703") // undefined_column — schema drift
                | Some("22001") // string_data_right_truncation — over VARCHAR(255) (AD-1)
                | Some("23514") // check_violation
        ),
        sqlx::Error::ColumnDecode { .. } | sqlx::Error::Decode(_) => true,
        _ => false,
    }
}
```

```rust
fn offset_error(err: sqlx::Error) -> OffsetStoreError {
    let text = err.to_string();
    if is_fatal(&err) { OffsetStoreError::Fatal(text) } else { OffsetStoreError::Transient(text) }
}
// `dedup_error` estructuralmente idéntico → DedupStoreError::{Fatal, Transient}
```

**Criterios**: (a) ambos SPIs declaran la misma partición de dos variantes
(`offset.rs:42-49`, `dedup.rs:9-18`) y ninguno define qué fallas van a cuál — dejarlo al
azar en cada sitio de llamada es como un adaptador termina reintentando para siempre contra
una tabla inexistente; (b) el predicado es puro y no toma pool, así que es la **única
superficie genuinamente comprobable por prueba unitaria** que agrega este cambio (AD-12),
igual que la forma del propio `#[cfg(test)]` de `reservation.rs`, que prueba únicamente
ayudantes puros; (c) vive en `postgres/mod.rs` porque dos archivos de adaptador lo
necesitan y ese módulo ya hospeda un ítem compartido interno al crate
(`pub(crate) use resolve_tenant`, `mod.rs:16-21`) — una definición, no una copia por
archivo.

### AD-9 — Dos archivos de adaptador, ambos reexportados; sin método `probe()`

**Decisión**: `read_side_offset.rs` (`PostgreSQLOffsetStore`) y `read_side_dedup.rs`
(`PostgreSQLDedupStore`), cada uno `pub struct { pool: PgPool }` con
`pub fn new(pool: PgPool)`, un `Debug` manual que imprime solo el pool (igual que
`snapshot.rs:31-37`, `reservation.rs:75-81`), `is_durable() -> true` incondicional
(`snapshot.rs:52-54`, `event_store.rs:126-128`), y `pub use` desde `postgres/mod.rs`
(IS-4).

**Criterios**: un archivo por almacén es la forma existente (`event_store.rs`,
`snapshot.rs`, `reservation.rs`, `repository.rs`), y los dos nombres con prefijo evitan que
se lean como almacenes del lado de escritura en un directorio que ya tiene un
`snapshot.rs`.

**Sin `probe()`, deliberadamente.** `reservation.rs:537-561` consulta su tabla real en vez
de `SELECT 1` precisamente para que una migración faltante se descubra en el chequeo de
disponibilidad y no en la primera escritura — correcto, y alcanzable allí porque
`OperationReservationStore` declara el método. `OffsetStore`/`DedupStore` no declaran
método de salud, y agregar uno inherente que nadie llama es andamiaje. La propiedad que
`probe()` protege se preserva en cambio mediante AD-8: una migración no aplicada aflora
como `42P01` → `Fatal`, distinguible de una caída transitoria. Una sonda de disponibilidad
real pertenece al cambio que agregue el método al SPI, no a este.

### AD-10 — Cableado en la app de referencia: constructor `postgres(pool)`, y el pool se clona *antes* de `EntityEventStores::open` (resuelve EC-2)

**Decisión** — `examples/reference-app/src/read_side/mod.rs`, junto a `in_memory()` y
`fake_durable()`:

```rust
/// Durable, and the only pair a `Profile::Production` composition can register
/// and satisfy (PROD-014B IS-5). `pool` must already have had
/// `ego_persistence::postgres::migrations::run` applied — migrations `013`/`014`
/// create the two tables these stores write to.
///
/// Adoption constraint (PROD-014B L-3): safe only where exactly one writer per
/// `(projection_id, tag, tenant)` exists. Two replicas of this projection are
/// outside the guarantee and nothing here detects it — see PROD-014C.
pub fn postgres(pool: PgPool) -> Self {
    Self {
        offset: Arc::new(PostgreSQLOffsetStore::new(pool.clone())),
        dedup: Arc::new(PostgreSQLDedupStore::new(pool)),
    }
}
```

**Decisión** — `examples/reference-app/src/main.rs:73-114`, con el orden explícito:

```rust
let pool = PgPoolOptions::new()…connect(&config.database.url).await?;
ego_persistence::postgres::migrations::run(&pool).await?;   // 013/014 aplicadas aquí
let read_side_progress = ReadSideProgressStores::postgres(pool.clone());  // EC-2: antes del move
let stores = EntityEventStores::open(pool).await?;          // pool movido
…
build_runtime_with(…, Some(read_side_progress))?;           // era None + comentario "PROD-014A F-1"
```

**Criterios**: (a) EC-2 — `open` toma el pool por valor, así que el clon debe preceder a la
línea 78; `PgPool` es `Clone` sobre un `Arc` compartido, de modo que ambos almacenes y los
almacenes de eventos comparten un solo pool de conexiones en vez de abrir un segundo;
(b) `migrations::run` ya precede a ambos (línea 77), así que no cambia ningún orden ni se
agrega una nueva llamada de migración; (c) el host ya es `Profile::Production`
(`EntityEventStores::open`), así que `Some(pair)` ahora atraviesa
`validate_read_side_progress_profile` sobre un backend durable real por primera vez — la
lógica de la compuerta queda intacta (SC-5, D-6); (d) `build_runtime_with` ya enhebra un
par declarado hacia el registro y hacia `ProjectionSpec` desde un mismo valor
(`lib.rs:793`, `:869-875`), así que IS-6 es un cambio de un argumento, no plomería nueva.

### AD-11 — Sin retención, y la afirmación de crecimiento es una línea operacional

**Decisión**: sin TTL, sin purga, sin desalojo, sin particionado, sin índice de retención
(D-4, OOS-3). `projection_dedup` crece monótonamente, lineal con la cantidad de eventos
únicos procesados (L-4). El rustdoc de los adaptadores lleva una nota operacional: el
conteo de filas es una señal a observar, y el disparador de escalamiento y el mecanismo
pertenecen a **F-2**.

**Criterios**: la retención necesita un horizonte, y la regla que ata la eliminación de
dedup al avance del offset no existe hoy en ninguna capa (`scheduler.rs`, `runner.rs`,
`session.rs` no contienen ninguna — Q2). Entregar aquí un camino de limpieza inventaría esa
regla dentro de un adaptador de persistencia, donde ningún llamador podría verla. El
`effect_dedup` de `crates/effect-store` fija el precedente del espacio de trabajo: la
retención se posee por separado (`effect-store/src/postgres/mod.rs:285-356`).

### AD-12 — El comportamiento se prueba solo contra PostgreSQL real en `integration-tests/`; el crate conserva exactamente una superficie de prueba unitaria pura

**Decisión**: `is_fatal` (AD-8) es la única prueba unitaria `#[cfg(test)]` nueva en
`crates/persistence`. Toda afirmación de comportamiento se prueba en
`integration-tests/tests/infrastructure/read_side_progress_postgres.rs` vía
`ego_integration_tests::isolated_database()`. Ninguna prueba construye un `PgPool` dentro de
una prueba unitaria de `crates/`, incluido `connect_lazy` (D-8, SC-10).

**Criterios**: las Reglas 1 y 2 de `ego-rs-testing` prohíben un pool real en una prueba
unitaria, y la excepción arquitectónica documentada de la Regla 3 es exactamente el espacio
de trabajo `integration-tests/` de nivel raíz con `isolated_database()` por prueba. El
propio bloque `#[cfg(test)]` de `reservation.rs` (`:608-669`) prueba solo ayudantes puros de
conversión — la misma línea que traza este diseño. `is_durable()` devuelve una constante,
pero se afirma en la suite de conformidad, donde existe un almacén real, en vez de a través
de un pool traído a la existencia para una sola aserción.

---

## Puntos de Integración

| Frontera | Dirección | Mecanismo | Verificado en |
|---|---|---|---|
| `ego-domain` → `ego-persistence` | arriba | ya es la única dependencia del crate | `crates/persistence/Cargo.toml:6-15` |
| adaptadores → borrado a `Arc<dyn …>` | afuera | impls genéricas de reenvío `Arc<T>` de PROD-014A, heredadas gratis | `offset.rs:91-119`, `dedup.rs:59-86` |
| `is_durable()` → compuerta `Profile::Production` | adentro | `validate_read_side_progress_profile` sin cambios | `builder.rs:879-891`; D-6 |
| `ego-persistence` → `reference-app` | arriba | dependencia ya declarada | `reference-app/Cargo.toml:44,51` |
| par → registro **y** `ProjectionSpec` | afuera | un valor, dos destinos, ya cableado | `lib.rs:793-796`, `:869-875` |
| esquema → runtime | adentro | ejecutor existente `include_str!` + `raw_sql` | `migrations.rs:43-57`; AD-2 |
| adaptadores → planificador / SPI | **ninguna** | no se agrega camino, no existe ninguno | OOS-1, OOS-2 |

Cero plomería nueva: cada cruce anterior ya existe.

## Estrategia de Pruebas

TDD estricto — la suite de conformidad se escribe en ROJO, contra tipos que aún no
compilan, antes que cualquier cuerpo de adaptador. Cada aserción de error nombra la variante
específica, nunca `is_err()`.

| Nivel | Ubicación | Qué prueba |
|---|---|---|
| Unitaria | `crates/persistence/src/postgres/mod.rs` `#[cfg(test)]` | AD-8: `42P01`/`42703`/`22001`/`23514` y `ColumnDecode`/`Decode` clasifican como `Fatal`; timeout de pool, E/S y errores de protocolo clasifican como `Transient`. Función pura, valores `sqlx::Error` construidos, **sin pool** |
| Unitaria | `crates/persistence/src/postgres/migrations.rs` (pruebas existentes, sin código nuevo) | AD-2: `013`/`014` quedan registradas y ordenadas — la prueba bidireccional de registro existente falla si un `.sql` se entrega sin registrar |
| Integración (PG real) | `integration-tests/tests/infrastructure/read_side_progress_postgres.rs`, `isolated_database()` | **SC-1** supervivencia al reinicio: escribir un offset, **descartar el almacén y su pool**, abrir un pool *nuevo* contra la misma base, reconstruir el almacén y leer `N` de vuelta — el valor en proceso nunca es la evidencia (R-3). **SC-2** `(projection_id, tag, tenant)` no escrito → `None`, y la lectura del inquilino B nunca devuelve el offset del inquilino A. **SC-3** `mark_seen` dos veces secuencial *y* dos veces concurrente (`tokio::join!`) → ambos `Ok`, `SELECT COUNT(*)` es exactamente `1`, `seen()` es `true`. **SC-4** el mismo `event_id` bajo otro inquilino, mismo `(projection_id, tag)` → ya visto. **SC-5** ambos `is_durable()` son `true`, y `build_runtime_with(…, Some(ReadSideProgressStores::postgres(pool)))` construye bajo `Profile::Production` — lo cual es también **SC-6**, la ruta de producción de referencia probada como usable. Además: `read_offset` contra una base sin migración aplicada devuelve `Fatal`, no `Transient` (AD-8, y la propiedad que de otro modo habría cubierto `probe()` — AD-9) |
| — | `examples/reference-app/tests/` | Nada agregado. Esos binarios no alcanzan ningún servicio externo; la afirmación sobre el cableado de producción se prueba en la fila anterior, a través de `build_runtime_with` y no rodeándolo |

Dos propiedades son propiedades del diff, no de una prueba, y se verifican leyendo el
cambio: **SC-7** (ninguna interpolación en ningún lado; cada `$N` ligado, cada sentencia de
offset liga `tenant`) y **SC-11** (`crates/domain/src/read_side/`, la compuerta y el
registro de `crates/service-sdk`, y `crates/runtime/src/read_side/scheduler.rs` no aparecen
en ninguna lista de archivos de este documento).

## Matriz de Amenazas

N/A — sin frontera de enrutamiento, comando de shell, subproceso, automatización de
VCS/PR, clasificación de archivos ejecutables ni integración de procesos. Este cambio
agrega dos adaptadores SQL, dos archivos DDL, un constructor en el host y una suite de
pruebas; no se invoca ningún proceso externo y ningún archivo se ejecuta ni se clasifica.

La superficie de seguridad aplicable son las Reglas 1 y 2 de `ego-rs-security`, y queda
cerrada por construcción: cada valor es un `$N` ligado (AD-3, AD-4, AD-5), ningún
identificador ni valor se interpola en el texto SQL, y no se usa ninguna excepción de lista
blanca — a diferencia del `CREATE DATABASE` de `isolated_database()`, este cambio no
interpola absolutamente nada. La Regla 2 queda satisfecha para `projection_offsets` (cada
sentencia liga `tenant`) y explícitamente acotada para `projection_dedup` en AD-7.

## Migración / Despliegue

Dos migraciones aditivas `CREATE TABLE IF NOT EXISTS`, no referenciadas por nada y que no
referencian nada. Ninguna tabla, columna, índice o consulta existente cambia. El orden de
despliegue es el existente: `migrations::run` ya precede a toda construcción de almacén en
`main.rs` y en la base plantilla de la suite de integración.

El rollback es el de la propuesta, sin cambios: borrar ambos adaptadores y sus
reexportaciones, borrar `013`/`014` y sus entradas de registro, borrar la suite de
conformidad, quitar `ReadSideProgressStores::postgres`, restaurar `main.rs` a `None`. Las
dos tablas pueden eliminarse o dejarse; no existe dato escrito antes de este cambio, y el
estado escrito entre despliegue y reversión degrada una proyección de vuelta al
comportamiento actual de reproducir desde cero, en lugar de corromper nada.

## Trazabilidad

| Ítem de la propuesta | Resuelto por | Nota |
|---|---|---|
| IS-1, D-1, D-2, D-3 | AD-1, AD-3, AD-4 | inquilino `NOT NULL`; upsert plano |
| IS-2, D-1, G-2 | AD-1, AD-5 | la clave primaria **es** la identidad UNIQUE |
| IS-3, D-5, R-4 | AD-2 | `013`/`014`; las pruebas de registro existentes las cubren |
| IS-4 | AD-9 | un archivo por almacén, ambos reexportados |
| IS-5 | AD-10 | `ReadSideProgressStores::postgres(pool)` |
| IS-6, SC-6 | AD-10 + **EC-2** | clonar antes de `EntityEventStores::open` |
| IS-7, D-8, SC-10 | AD-12 | `isolated_database()`; solo una prueba unitaria pura |
| IS-8, SC-8, SC-12, L-1/L-2/L-3 | **AD-6** | la garantía es un-solo-escritor-por-`(projection_id, tag, tenant)` |
| D-4, L-4, OOS-3 | AD-11 | sin cota; una línea operacional; F-2 posee la retención |
| D-6, OOS-1, SC-5 | AD-9, AD-10 | `is_durable() -> true`; compuerta intacta |
| D-7, OOS-2, F-1, SC-9 | AD-6 | PROD-014C nombrado; punto de enganche identificado, no diseñado |
| L-5, Q4 | AD-3 + **EC-1** | deja sin efecto la lectura del `UPDATE` condicional de `explore.md` §2 |
| G-1, SC-1, R-3 | AD-12 | reinicio probado con un pool nuevo, no con un valor en proceso |
| G-4, SC-2, SC-7 | AD-4 | `tenant` ligado en cada sentencia de offset |
| Q1, Q3, R-6 | AD-1 | identidad y `NOT NULL`; relajar a anulable sigue siendo una migración hacia adelante |
| Q5, R-1 | AD-6, AD-7 | contabilidad vs ejecución enunciado como tabla, no como nota al pie |
| — | AD-8 | nuevo: la partición `Transient`/`Fatal` estaba indefinida en ambos SPIs |
| R-5 | — | `sdd-tasks` posee el pronóstico de 400 líneas; aquí no se anticipa |

## Preguntas Abiertas

- [ ] Criterio 5 de AD-1 — `VARCHAR(255)` sigue la convención y rechaza (SQLSTATE `22001`)
      en lugar de truncar un identificador demasiado largo. Ningún sitio de llamada produce
      uno hoy; confirmar que el techo es aceptable en vez de cambiar este par a `TEXT`.
- [ ] AD-9 — sin `probe()`. `reservation.rs` tiene uno porque su puerto lo declara; aquí el
      caso de migración no aplicada queda cubierto por la clasificación `Fatal`. Confirmar
      que no se espera ninguna superficie de disponibilidad de estos adaptadores en este
      cambio.
