# Diseño: PROD-013 — Endurecimiento de la Composición de Producción

> Companion de revisión en español. Fuente canónica: `design.md` (identificadores 1:1).
>
> **Entradas**: `proposal.md` (D-1 … D-8, IS-1 … IS-12, R-1 … R-7, SC-1 … SC-11) y
> `explore.md`. Este documento decide el **cómo**, nunca el **qué** — excepto donde
> leer el código real falsificó una premisa sobre la que se apoyaba el *qué*. Existen
> dos casos (§Correcciones por Evidencia) y ambos se exponen en lugar de rodearse en
> silencio, porque diseñar contra una premisa que se sabe falsa entrega un cambio roto
> con un rastro documental impecable.
>
> **Baseline verificado**: `develop` @ `a740d34`.

## Enfoque Técnico

Un enum `Profile` vive en el crate más bajo que lo necesita (`persistent-entity`) y se
reexporta hacia arriba. Un único predicado compartido — `require_durably_configured` —
es el único lugar donde se decide "producción declarada + capacidad no configurada *con
durabilidad* = rechazo"; ambos builders lo invocan en lugar de reformular la regla cada
uno. La señal de durabilidad en sí proviene de una declaración de capability mínima en
el propio trait de cada store (`is_durable()` para event/snapshot store, reutilizando el
`EffectStoreCapabilities.durable` ya existente de PROD-002 para el effect store) —
nunca de si un store simplemente estaba *presente*. Hay dos builders
porque las capacidades viven en dos crates y el límite de capas es de una sola
dirección: `EntityRuntimeBuilder` para los stores de eventos y snapshots,
`RuntimeBuilder` para el effect store. `EntityRuntimeBuilder` gana un hermano
`try_build()` que espeja exactamente la forma validar-antes-de-delegar de PROD-012, y
`build()` conserva su firma infalible y panickea — así los 67 call sites existentes
siguen compilando.

La reference app luego demuestra que el mecanismo está vivo y no solamente disponible:
su composición de producción declara `Profile::Production` **a través del tipo que ya
existe para declarar su elección de stores**, de modo que la declaración no puede
olvidarse de forma independiente de los stores durables que describe.

---

## Correcciones por Evidencia

Ambas se encontraron leyendo el código al que apunta el proposal. Ninguna es una
preferencia de diseño; cada una es una corrección factual con evidencia file:line, y
cada una cambia lo que el cambio debe hacer.

### EC-1 — La premisa de costo de D-2 es falsa: existen 15 call sites con configuración parcial, incluida la raíz de composición de producción de la reference app

D-2 afirma que el chequeo de configuración parcial independiente del profile tiene
"costo cero en blast radius: ningún call site actual hace esto — todos configuran ambos
stores o ninguno (explore §4)".

Medido sobre `develop @ a740d34`: `with_event_store` tiene 18 call sites,
`with_snapshot_store` tiene 4. Solo tres cadenas configuran ambos. **Quince cadenas
configuran exactamente uno** y serían rechazadas de plano por un chequeo incondicional:

| # | Sitio | Configurado |
|---|---|---|
| 1 | `examples/reference-app/src/lib.rs:502` (`observed_entity_runtime`) | solo event |
| 2 | `examples/reference-app/tests/register_user_multi_aggregate_recovery.rs:341` | solo event |
| 3 | `examples/reference-app/tests/register_user_multi_aggregate_recovery.rs:347` | solo event |
| 4 | `crates/persistent-entity/tests/receipt_written_in_unit_of_work.rs:284` | solo event |
| 5 | `crates/persistent-entity/tests/receipt_written_in_unit_of_work.rs:454` | solo event |
| 6 | `crates/persistent-entity/tests/real_actor_path_tests.rs:126-130` | solo event |
| 7 | `crates/persistent-entity/tests/guaranteed_completion_tests.rs:164` | solo event |
| 8 | `crates/persistent-entity/tests/guaranteed_completion_tests.rs:547` | solo event |
| 9 | `crates/persistent-entity/tests/guaranteed_completion_tests.rs:1037` | solo event |
| 10 | `crates/persistent-entity/tests/receipt_outcome_metric.rs:345` | solo event |
| 11 | `crates/persistent-entity/tests/receipt_outcome_metric.rs:367` | solo event |
| 12 | `crates/persistent-entity/tests/activation_ordering_tests.rs:44` | solo event |
| 13 | `crates/persistent-entity/tests/receipt_gating.rs:265` | solo event |
| 14 | `integration-tests/tests/infrastructure/single_aggregate_crash_recovery_postgres.rs:284` | solo event |
| 15 | `crates/persistent-entity/tests/real_actor_path_tests.rs:210-214` | **solo snapshot** |

El sitio 1 es la raíz de composición de producción de la reference app. El sitio 15
prueba que la asimetría corre en ambas direcciones. Configuran ambos:
`persistence_failure_tests.rs:172-173`, `210-211`, `238-239`.

El proposal es, por tanto, internamente contradictorio tal como está escrito. IS-5 y
SC-6 exigen rechazo "en todos los profiles"; IS-8 y SC-7 exigen que "los 67 call sites
existentes compilen y pasen sin modificación". Con 15 sitios parciales, exactamente uno
de esos pares puede sostenerse. Resuelto en **AD-7**.

### EC-2 — El effect store sí tiene un fallback silencioso a in-memory, exactamente igual que los otros dos

Explore §1.4 afirma que el campo del effect store es "un `Option<...>` simple **sin**
fallback `unwrap_or_else` en ninguna parte (confirmado por grep — cero construcción de
default in-memory para el effect store)" y concluye que el riesgo es una falla diferida
en el primer uso, no volatilidad silenciosa. D-3, IS-4 y SC-3 heredan ese encuadre.

`crates/service-sdk/src/runtime/builder.rs:804-817` dice lo contrario:

```rust
let effect_acceptor_impl = if self.effect_executors.is_empty() {
    None
} else {
    let (state_store, dedup_store) =
        match (self.effect_state_store, self.effect_dedup_store) {
            (Some(state_store), Some(dedup_store)) => (state_store, dedup_store),
            _ => {
                let store = Arc::new(InMemoryEffectStore::new());   // <- línea 811
                ...
```

El fallback es un brazo de `match`, no un `unwrap_or_else`, y por eso el grep no lo
vio. Su propio doc comment lo dice sin rodeos en `builder.rs:493-495`: "sin esta
llamada `build()` sigue construyendo `InMemoryEffectStore` exactamente como antes,
siempre que haya un executor registrado".

Esta corrección vuelve el diseño **más simple y más coherente**, no más difícil: las
tres capacidades gateadas comparten un único modo de falla idéntico — sustitución
silenciosa por almacenamiento volátil — así que "un gate, una regla" (D-3) es más
cierto de lo que el proposal afirmaba, no menos. Cambia dos cosas:

- La cláusula de SC-3 "y falla en el **bootstrap** en lugar del primer uso" describe un
  modo de falla que el effect store no tiene. Lo que realmente previene es la misma
  volatilidad silenciosa que SC-4. La redacción del spec debe seguir al código.
- El gate debe estar condicionado a que haya al menos un executor registrado
  (**AD-5**), ya que sin ninguno no se construye ningún store y nada es volátil.

---

## Mapa de Componentes

```
crates/domain                                     (traits que implementan ambos stores)
├── src/persistence/event_store.rs        MOD   + EventStore::is_durable() (default false)
└── src/persistence/snapshot.rs           MOD   + Snapshot::is_durable() (default false)
                                                   ↑ implementado por
crates/persistence                                (implementaciones Postgres)
├── src/postgres/event_store.rs           MOD   is_durable() -> true
└── src/postgres/snapshot.rs              MOD   is_durable() -> true
                                                   ↑ leído por
crates/persistent-entity                          (capa baja, sin dep de service-sdk)
├── src/profile.rs                        NUEVO Profile { Dev, Production }
│                                               require_durably_configured(...)  ← LA regla
├── src/error.rs                          MOD   + PersistenceCompositionError
└── src/builder.rs                        MOD   + .profile(), validate_persistence(),
                                                try_build(); build() valida+panickea
                                                ↑ depende de
crates/service-sdk                                (capa alta)
├── src/runtime/mod.rs                    MOD   pub use persistent_entity::profile::Profile;
├── src/runtime/runtime_builder.rs        MOD   + RuntimeError::PersistenceNotConfigured
│                                                (#[from] PersistenceCompositionError)
├── src/runtime/builder.rs                MOD   + .profile(), validate_persistence_profile()
│                                                invocado desde build()/try_build()
└── src/app/mod.rs                        MOD   + AppBuilder::profile() (delegación fina)
                                                CompositionError::Validation ya reenvía
                                                ↑ usado por
examples/reference-app
├── src/lib.rs                            MOD   EntityEventStores lleva profile + snapshot
│                                                stores; observed_entity_runtime pasa ambos
├── tests/production_profile_guard.rs     NUEVO Guarda de regresión lado Dev (IS-12)
└── (integration-tests/...postgres.rs)    MOD   una aserción de Production (IS-12)
```

## Flujo de Datos

```
Host                                  Framework                       Resultado
────                                  ─────────                       ─────────
EntityRuntimeBuilder::new()
  .profile(Production)
  .with_event_store(pg)          ──▶  validate_persistence()
  .with_snapshot_store(pg)              ¿is_durable()? × 2, luego
  .try_build()                          require_durably_configured × 2   ──▶  Ok  → EntityRuntime
                                                                            Err → PersistenceCompositionError
                                                                            (lo maneja el host; AD-6)

App::builder()
  .profile(Production)
  .effect_executor(...)          ──▶  RuntimeBuilder::validate_persistence_profile()
  .effect_store(pg)                     ¿capabilities().durable?, luego
                                         require_durably_configured × 1   ──▶  Ok  → App
  .build()                                                            Err → RuntimeError::PersistenceNotConfigured
                                                                            → CompositionError::Validation
```

---

## Decisiones de Arquitectura

### AD-1 — `Profile` vive en `crates/persistent-entity/src/profile.rs`, reexportado desde `service-sdk::runtime`

**Decisión**: un módulo dedicado nuevo de 20 líneas en el crate bajo, más una línea de
reexport.

```rust
// crates/persistent-entity/src/profile.rs
/// Lo que una composición declara sobre el despliegue para el cual se construye.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    /// El comportamiento de hoy, byte por byte. El almacenamiento volátil por
    /// omisión es válido aquí, porque para eso existen dev y test.
    #[default]
    Dev,
    /// Toda capacidad persistente que esta composición usa debe configurarse
    /// explícitamente. Nada volátil es alcanzable por omisión.
    Production,
}
```

```rust
// crates/persistent-entity/src/lib.rs
pub mod profile;

// crates/service-sdk/src/runtime/mod.rs  (junto al bloque pub use existente)
pub use persistent_entity::profile::Profile;
```

**Criterios**: (a) `service-sdk` depende de `persistent-entity`
(`crates/service-sdk/Cargo.toml:24`) y nunca al revés, así que el tipo compartido tiene
exactamente un hogar admisible; (b) un tipo, no dos — un `service_sdk::Profile` distinto
de un `persistent_entity::Profile` permitiría que un host declarara Production en el
`AppBuilder` y Dev en sus entity runtimes sin que nada objetara.

**Por qué un módulo dedicado y no dentro de `builder.rs`**: precedente exacto.
`IdempotencyEnforcementMode` — el tipo que todo este cambio espeja — vive en su propio
`crates/service-sdk/src/runtime/idempotency.rs`, no dentro de `builder.rs`, y se
reexporta desde `runtime/mod.rs:17`. `Profile` gobierna dos builders en dos crates, así
que `use persistent_entity::builder::Profile` lo describiría mal.

**Consecuencia**: los hosts escriben `use ego_service_sdk::runtime::Profile` (o
`persistent_entity::profile::Profile` cuando solo componen entity runtimes). `Default`
se derivea en lugar de escribirse a mano: a diferencia de `IdempotencyEnforcementMode`,
cuyo default necesitaba un párrafo explicando por qué es la variante *estricta*,
`Profile::Dev` es la permisiva y D-1 ya carga ese razonamiento.

### AD-2 — El rechazo es `PersistenceCompositionError`, en `crates/persistent-entity/src/error.rs`

**Decisión**: un enum `thiserror` nuevo en el módulo de errores existente, que lleva la
capacidad y la pista de arreglo como `&'static str`.

```rust
// crates/persistent-entity/src/error.rs
/// Una composición se declaró de producción y una capacidad persistente que
/// usa no tiene implementación configurada explícitamente.
///
/// Deliberadamente no es una variante de `EntityError`: `EntityError` reporta
/// qué salió mal mientras una entidad *ejecutaba* un comando. Esta reporta que
/// el runtime no debe construirse en absoluto, y nada que maneje una falla de
/// comando debería tener que considerarla.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PersistenceCompositionError {
    #[error(
        "Profile::Production is declared but no {capability} is configured — a \
         production composition must never fall back to volatile storage. \
         Configure one with {fix}, or state that this composition is not \
         production with .profile(Profile::Dev)"
    )]
    NotConfigured {
        /// La capacidad sin implementación configurada.
        capability: &'static str,
        /// La llamada exacta que lo arregla.
        fix: &'static str,
    },
}
```

**Criterios**: (a) `persistent-entity` no puede tomar prestado
`RuntimeError`/`CompositionError` (R-4, verificado: no hay dependencia a `service-sdk`);
(b) `thiserror = "1.0"` ya es dependencia (`crates/persistent-entity/Cargo.toml:11`),
así que no hay dependencia nueva; (c) IS-7 exige nombrar tanto la capacidad faltante
como la llamada exacta que la arregla — los campos `&'static str` entregan ambas
manteniendo una sola variante, igual que `RuntimeError::DependencyNotFound` ya lleva
`type_name: &'static str` más una pista de arreglo (`runtime_builder.rs:1481-1493`).

**Segunda opción, rechazada**: una variante nueva `EntityError::PersistenceNotConfigured`.
`EntityError` es un enum con `Display` escrito a mano cuyas variantes describen todas un
resultado de ejecución de comando (`crates/persistent-entity/src/error.rs:9-50`), y
cada `match` sobre él — incluidos los exhaustivos en el código de actor y recovery —
ganaría un brazo muerto para una condición que solo puede ocurrir antes de que exista
cualquier actor.

**Rechazado de plano**: una variante por capacidad (tres variantes). El texto del
mensaje difiere en dos `&'static str`; tres variantes significan tres cadenas `#[error]`
que mantener consistentes, que es exactamente la deriva que SC-8 existe para prevenir.

### AD-3 — Un único predicado compartido, `require_durably_configured`, decide rechazar-o-permitir; cada capacidad aporta su propia señal de durabilidad (IS-6 / SC-8)

**Decisión**: la regla es una función libre junto a `Profile`, y todo gate la invoca.
Es el único lugar del workspace donde las palabras "Production" y "no configurado con
durabilidad" se encuentran. El booleano que recibe DEBE representar durabilidad, nunca
mera presencia — `Some(store_volátil).is_some()` es `true`, y ese es exactamente el
error que el nombre del argumento de este predicado existe para volver difícil de
cometer por accidente.

```rust
// crates/persistent-entity/src/profile.rs
use crate::error::PersistenceCompositionError;

/// La única definición de la regla de PROD-013.
///
/// Una función libre y no un método de alguno de los builders, porque las tres
/// capacidades gateadas viven en dos crates que no pueden compartir un builder:
/// los stores de eventos y snapshots son de `EntityRuntimeBuilder`, el effect
/// store es de `RuntimeBuilder`, y `persistent-entity` no puede ver
/// `EffectStateStore`. Reformular la regla una vez por crate crearía exactamente
/// el segundo chequeo paralelo que SC-8 prohíbe; pasar los tres hechos variables
/// como argumentos la mantiene única.
///
/// `durably_configured`, no `configured`: cada call site DEBE calcular esto a
/// partir de la propia declaración de durabilidad de la capacidad (abajo), nunca
/// solo de `.is_some()` — presencia y durabilidad son propiedades distintas.
pub fn require_durably_configured(
    profile: Profile,
    durably_configured: bool,
    capability: &'static str,
    fix: &'static str,
) -> Result<(), PersistenceCompositionError> {
    match profile {
        Profile::Production if !durably_configured => {
            Err(PersistenceCompositionError::NotConfigured { capability, fix })
        }
        _ => Ok(()),
    }
}
```

**Criterios**: (a) SC-8 exige un predicado compartido único como fuente de verdad para
la *decisión* de rechazar/permitir "a través de las tres capacidades"; (b) debe seguir
siendo cierto a través de un límite de capas que ninguna función puede cruzar si
inspecciona campos del builder directamente — por eso la función toma la *respuesta*
(`durably_configured: bool`) y no el builder, y cada superficie de composición conserva
solo el chequeo de capacidad de una línea que únicamente ella puede hacer;
(c) `persistent-entity` genuinamente no puede ver el effect store: `EffectStateStore` y
`EffectDedupStore` son tipos de `ego-runtime` alcanzados vía `service-sdk`
(`builder.rs:12`), e importarlos hacia abajo invertiría la dependencia que
`xtask verify-layers` hace cumplir.

**Esta es la respuesta honesta a la pregunta que el proposal difirió** ("¿hay una forma
de unificarlo en un solo punto pese al layering?"): sí, pero solo extrayendo la
*decisión* en lugar de la *inspección*. La decisión es la parte que podría derivar y la
parte de la que trata SC-8. La inspección es la respuesta de dos partes de abajo.

**De dónde viene la señal de durabilidad en sí** (la propiedad que este predicado puede
recibir pero no puede fabricar): una declaración de capability mínima en el propio
trait de cada store, espejando el patrón que PROD-002 ya estableció para el effect
store.

```rust
// crates/domain/src/persistence/event_store.rs — EventStore<E>, y el método
// estructuralmente idéntico en crates/domain/src/persistence/snapshot.rs's Snapshot
fn is_durable(&self) -> bool {
    false   // default: honesto para toda implementación existente y de terceros
}
```

`InMemoryEventStore`/`InMemorySnapshotStore`/`NoopEventStore`/`NoopSnapshotStore`
heredan este default sin tocar — nada de ellos cambia. `PostgreSQLEventStore`/
`PostgreSQLSnapshotStore` lo sobreescriben a `true`. El effect store no necesita ningún
método de trait nuevo en absoluto: `EffectStateStore::capabilities() ->
EffectStoreCapabilities { durable, ... }` ya existe (PROD-002 AD-3,
`crates/runtime/src/effects/store.rs:238-244`), tiene default `durable: false`, y toda
implementación Postgres del effect store ya lo sobreescribe a `true`
(`crates/effect-store/src/postgres/mod.rs:379-386`, `:690-697`) — el AD-5 de abajo
reutiliza `capabilities().durable` directamente.

**Por qué un método de trait, y no un downcast, un tipo marcador, o un registro
separado**: (a) un downcast a un tipo concreto `InMemoryEventStore`/
`PostgreSQLEventStore` haría que el gate no pudiera reconocer ninguna implementación
durable de terceros — tendría que enumerar por nombre cada tipo concreto del workspace,
para siempre; un método de trait lo responde la propia implementación, así que un store
durable de un crate externo simplemente lo sobreescribe y queda reconocido sin ningún
cambio del lado del gate. (b) Ningún método de trait existente necesitó cambiar firma
ni comportamiento — esto es aditivo, así que nada que ya implemente
`EventStore`/`Snapshot` se rompe al heredar el default. (c) Un solo `bool` es
proporcional: ninguno de los dos traits declara hoy ninguna otra capability, así que
una struct `Capabilities` completa (como ya tiene el effect store, para cuatro
preocupaciones *distintas* — durabilidad, seguridad de concurrencia local, seguridad
multi-nodo, soporte de leases) sería una ceremonia de un solo campo copiada sin
motivo; si alguna vez se necesita acá una segunda preocupación de capability, ese es el
momento de introducir una struct, no antes.

**Consecuencia**: los call sites por capacidad se vuelven declaraciones de hecho —
calculadas a partir de la durabilidad, nunca de la presencia — y las cadenas de
capacidad/arreglo viven en el sitio dueño de la llamada que se recomienda:

```rust
// crates/persistent-entity/src/builder.rs
fn validate_persistence(&self) -> Result<(), PersistenceCompositionError> {
    require_durably_configured(
        self.profile,
        self.event_store.as_ref().is_some_and(|s| s.is_durable()),
        "event store", "EntityRuntimeBuilder::with_event_store(store)",
    )?;
    require_durably_configured(
        self.profile,
        self.snapshot_store.as_ref().is_some_and(|s| s.is_durable()),
        "snapshot store", "EntityRuntimeBuilder::with_snapshot_store(store)",
    )
}
```

El event store se chequea primero, deliberadamente: cuando faltan ambos, quien llama ve
el que con mucha más probabilidad quería configurar, y PROD-012 estableció que un
rechazo reporta la primera violación y no una lista
(`try_build_fails_before_startup_when_declared_entity_dependency_is_missing`).

**Nota de revisión**: un borrador anterior de esta sección calculaba el argumento a
partir de `self.event_store.is_some()` — presencia, no durabilidad — lo cual un revisor
marcó correctamente antes de que se implementara nada: `Some(InMemoryEventStore::new())`
y `Some(PostgreSQLEventStore::open(pool))` son indistinguibles bajo `.is_some()`, así
que `Profile::Production` habría aceptado un store volátil cableado explícitamente.
Cerrado acá, en esta misma decisión, antes de que WU2 la implemente — no como un parche
posterior.

### AD-4 — `EntityRuntimeBuilder` gana `try_build()`, espejando PROD-012 exactamente (resuelve R-3)

**Decisión**: espejar `crates/service-sdk/src/runtime/builder.rs:740-771` y
`:1088-1092` forma por forma. Sin desviación, porque ninguna se justifica.

```rust
// crates/persistent-entity/src/builder.rs

/// Consume el builder y produce un [`EntityRuntime`].
///
/// # Panics
///
/// Panickea cuando se declara [`Profile::Production`] y una capacidad
/// persistente gateada no tiene implementación configurada.
///
/// Un panic y no un `Result`, porque esta firma es la que ya invocan los 67
/// call sites existentes, y porque la alternativa es peor que una parada
/// ruidosa: un runtime que declara producción y escribe silenciosamente cada
/// evento en memoria de proceso los pierde en el próximo reinicio, y no
/// reporta nada. El bootstrap es el momento más barato para rechazar.
///
/// [`Self::try_build`] devuelve la misma condición como error estructurado.
pub fn build(self) -> EntityRuntime<E> {
    if let Err(err) = self.validate_persistence() {
        panic!("{err}");
    }
    /* ...cuerpo existente, sin cambios... */
}

/// Consume el builder y produce un [`EntityRuntime`], devolviendo el rechazo
/// del gate de profile en lugar de panickear.
pub fn try_build(self) -> Result<EntityRuntime<E>, PersistenceCompositionError> {
    // Antes de delegar, no después. `build` panickea con esta condición, así
    // que chequear después significaría que este método nunca podría devolver
    // el error que existe para devolver — el panic ya habría desenrollado.
    self.validate_persistence()?;
    Ok(self.build())
}
```

**Criterios**: (a) el proposal nombra esta plantilla como referencia y R-3 califica el
ordenamiento como load-bearing — lo es, y por la razón idéntica, así que el comentario
lo dice con las mismas palabras; (b) la firma de `build()` es load-bearing en 67 call
sites de 25 archivos (reverificado, R-2 saldado: `EntityRuntimeBuilder::new()` → 67
ocurrencias / 25 archivos en `a740d34`); (c) un revisor que conoce PROD-012 no necesita
un segundo patrón.

**Razones para desviarse, consideradas y ausentes**:

- *Cambiar `build()` para que devuelva `Result` en vez de agregar un hermano.* Rompe los
  67 sitios y viola IS-8/SC-7. Es la migración del Approach C disfrazada (AD-11).
- *Agregar solo `try_build()` y dejar `build()` sin validar.* Entonces
  `Profile::Production` en una llamada a `build()` se acepta en silencio y el gate es
  decorativo — exactamente el fail-open que el cambio existe para cerrar. PROD-012 lo
  rechazó por la misma razón.
- *Deprecar `build()`.* Un warning de deprecación en 67 sitios es ruido sobre una firma
  que es correcta para `Profile::Dev`, que es la mayoría de ellos.

**Diferencia con PROD-012, declarada porque es real**: `RuntimeBuilder::try_build` toma
`mut self` y hace más que validar — corre los validadores de `Injectable` después de
delegar. `EntityRuntimeBuilder::try_build` toma `self` y solo valida, así que es una
versión estrictamente más pequeña de la misma forma. No necesita `mut`.

### AD-5 — El gate del effect store vive en `RuntimeBuilder`, está condicionado a un executor registrado, y cruza hacia arriba por `RuntimeError`

**Decisión**: un segundo validador en `RuntimeBuilder`, invocado desde `build()` y
`try_build()` en el mismo orden en que ya lo está `validate_idempotency`, gateado a que
haya al menos un executor registrado.

```rust
// crates/service-sdk/src/runtime/builder.rs
fn validate_persistence_profile(&self) -> Result<(), RuntimeError> {
    // Condicionado a un executor registrado porque sin ninguno no se construye
    // ningún effect store (ver el gate `effect_acceptor_impl` de `build()`) —
    // no hay almacenamiento volátil que rechazar. Exigir uno de todos modos
    // forzaría a todo host de producción que no describe efectos externos a
    // registrar un store que nunca lee ni escribe.
    if self.effect_executors.is_empty() {
        return Ok(());
    }
    require_durably_configured(
        self.profile,
        self.effect_state_store
            .as_ref()
            .is_some_and(|s| s.capabilities().durable),
        "effect store",
        "RuntimeBuilder::with_effect_store(store) (or AppBuilder::effect_store(store))",
    )?;
    Ok(())
}
```

```rust
// crates/service-sdk/src/runtime/runtime_builder.rs — RuntimeError
/// Una composición de producción dejó una capacidad persistente sin
/// configurar (PROD-013). Envuelve el rechazo del crate inferior en lugar de
/// reformularlo: `persistent-entity` es dueño de la regla (AD-3) y esta capa
/// es dueña solo del cruce, exactamente como `CompositionError::Validation`
/// es dueña del cruce de este enum.
#[error("production composition validation failed: {0}")]
PersistenceNotConfigured(#[from] persistent_entity::error::PersistenceCompositionError),
```

**Criterios**: (a) EC-2 — el fallback en `builder.rs:811` es real, así que este gate
cierra volatilidad silenciosa real, no meramente una falla diferida; (b) `build()` y
`try_build()` deben coincidir, lo que la forma de validador único existente ya
garantiza; (c) `CompositionError::Validation(#[from] RuntimeError)` ya existe
(`app/error.rs:59`) y `AppBuilder::build()` ya mapea a través de él (`app/mod.rs:807`),
así que la superficie de `AppBuilder` cuesta cero plomería nueva — el "surfaces through
`AppBuilder` for free" del proposal se sostiene, verificado.

**Chequea `effect_state_store` solo, no ambos**: `with_effect_store` es la única forma
de poblar cualquiera de los dos campos y siempre setea ambos desde el mismo `Arc`
(`builder.rs:501-508`), invariante que `build()` ya asevera en `builder.rs:797-803`.
Chequear ambos implicaría un estado mixto que la API pública no puede expresar.

**Consecuencia**: `AppBuilder` gana un método delegante fino, igual a cómo delega
`effect_store` mismo (`app/mod.rs:562-578`):

```rust
pub fn profile(mut self, profile: Profile) -> Self {
    if self.pending_error.is_some() { return self; }
    self.runtime_builder = self.runtime_builder.profile(profile);
    self
}
```

`AppBuilder::profile` **no** propaga el profile a los entity runtimes registrados. No
puede: `AppBuilder::entity()` recibe un `Arc<EntityRuntime<E>>` ya construido, así que
el gate propio del entity runtime corrió antes de que `AppBuilder` lo viera. Su doc
comment debe decirlo, o un host asumirá razonablemente que una llamada cubre las tres
capacidades.

### AD-6 — Sin puente entre capas para el rechazo de event/snapshot (corrige una suposición del proposal)

**Decisión**: `PersistenceCompositionError` llega a `RuntimeError` solo por la vía del
effect store (AD-5). El rechazo de event/snapshot se devuelve a quien haya invocado
`EntityRuntimeBuilder::try_build()` y no va más lejos. No se agrega ningún
`From<PersistenceCompositionError>` para esa vía.

**Evidencia**: la sección Approach del proposal anticipa que el error de event/snapshot
"cruce el límite de capas exactamente como
`RuntimeError::OperationReservationStoreNotRegistered` ya lo hace una capa arriba". No
necesita cruzarlo, porque no hay ruta por la cual cruzar. Las instancias de
`EntityRuntime` las construye el **host**, no `RuntimeBuilder`:
`AppBuilder`/`RuntimeBuilder` solo reciben un `Arc<EntityRuntime<E>>` ya terminado vía
`with_entity` / `entity`. Confirmado en todo call site, p. ej.
`crates/service-sdk/src/runtime/builder.rs:3175-3177`
(`Arc::new(EntityRuntimeBuilder::<TestEvent>::new().build())` y luego
`.with_entity::<TestEntity>(entity_runtime)`), y en la reference app en
`lib.rs:649-658`.

`PersistenceCompositionError` implementa `std::error::Error` vía `thiserror`, así que el
`build_runtime_with(...) -> Result<BuiltRuntime, Box<dyn Error>>` de la reference app lo
absorbe con un `?` pelado y ninguna conversión.

**Criterios**: (a) escalón 1 de la escalera — un puente que nadie transita es superficie
especulativa; (b) agregarlo invita a un futuro lector a creer que `AppBuilder::build()`
puede reportar el rechazo de un entity runtime, lo que estructuralmente no puede;
(c) R-4 resulta más angosto de lo evaluado: un cruce, no dos, y el que existe usa el
precedente exacto citado.

### AD-7 — El chequeo incondicional de configuración parcial de D-2 se absorbe en el gate de Production

**Decisión**: no se entrega ningún chequeo separado independiente del profile. Bajo
`Profile::Production` un store faltante se rechaza (lo que subsume todo caso parcial);
bajo `Profile::Dev` nada cambia.

**Esto cambia el proposal y necesita la confirmación del arquitecto.** Se registra aquí
en lugar de absorberse en silencio, según el estándar de D-7 mismo: una migración debe
dimensionarse explícitamente, nunca doblarse dentro de "agregar un gate".

**Criterios**:

1. **Su costo declarado es falso.** EC-1: existen 15 call sites parciales en 8 archivos,
   no cero. La justificación entera del chequeo en D-2 es "costo cero en blast radius:
   ningún call site actual hace esto".
2. **El sitio 1 es la raíz de composición de producción de la reference app.**
   `observed_entity_runtime` (`examples/reference-app/src/lib.rs:502`) configura el
   event store y no el snapshot store, y *todo* entity runtime que la reference app
   construye — producción, in-memory y observado — pasa por ahí. Un chequeo
   incondicional vuelve inconstruible la composición propia de la reference app en
   todos los profiles, incluidas las variantes dev que IS-8 existe para proteger.
3. **El proposal no puede satisfacer las dos mitades de su propio contrato.** IS-5/SC-6
   ("ambos profiles") contra IS-8/SC-7 ("los 67 sitios compilan y pasan sin
   modificación"). Uno debe ceder, e IS-8/SC-7 es el que motivó D-1, el que sostiene el
   plan de rollback, y el que el arquitecto aprobó como la propiedad de blast radius
   cero del cambio.
4. **Gatearlo por profile en su lugar lo volvería redundante, no más barato.** Bajo
   `Profile::Production`, "exactamente uno configurado" ya es rechazado por la regla del
   faltante (AD-3), porque el faltante falta. Un chequeo parcial gateado por profile es
   código muerto por construcción.
5. **El único defecto real al que apuntaba D-2 se atrapa igual, en este cambio.** El
   sitio 1 — event store de Postgres cableado, snapshot store silenciosamente in-memory
   — es precisamente el error "event store cableado, snapshot store olvidado" que D-2
   describe, y AD-9 lo arregla porque AD-8 pone esa composición bajo
   `Profile::Production`.

**Alternativas consideradas**:

- *Mantenerlo incondicional y migrar los 15 sitios.* Agrega
  `.with_snapshot_store(Arc::new(Mutex::new(InMemorySnapshotStore::new())))` a 14
  cadenas de test que nunca hacen snapshot (la mayoría ya usa `NoSnapshot` como
  estrategia), no compra nada sobre el criterio 5, y rompe SC-7 de plano.
- *Advertir en lugar de rechazar.* `tracing` está disponible en `persistent-entity`
  (`Cargo.toml:12`), así que es barato. Rechazado: un warning que se dispara 15 veces en
  una corrida limpia de tests del workspace es ruido que entrena a los lectores a
  ignorarlo, y "loguear y continuar" es el contrato débil que este cambio reemplaza.

**Consecuencia**: IS-5, SC-6 y D-2 necesitan enmienda, y el tercer bloque `Given` del
criterio de aceptación ("cualquier composición, con o sin `Profile::Production`") se
angosta a Production. Nada más en el proposal depende de D-2.

### AD-8 — La reference app declara `Profile::Production` a través de `EntityEventStores`, no por un argumento de `build_runtime_with`

**Decisión**: `EntityEventStores` — el tipo que ya existe para que "la elección del
store de respaldo sea **declarada**, nunca por default" (`lib.rs:338`) — lleva el
profile. `EntityEventStores::open(pool)` produce `Profile::Production`;
`EntityEventStores::in_memory()` produce `Profile::Dev`. El campo del profile es
privado, y esos dos constructores son la única entrada.

```rust
pub struct EntityEventStores {
    pub org: Arc<dyn EventStore<OrganizationEnsured> + Send + Sync>,
    pub user: Arc<dyn EventStore<UserRegistered> + Send + Sync>,
    /// Ver AD-9.
    pub org_snapshot: Arc<Mutex<dyn Snapshot + Send>>,
    pub user_snapshot: Arc<Mutex<dyn Snapshot + Send>>,
    /// Privado, y seteado solo por los dos constructores: stores durables y una
    /// declaración de producción son una sola decisión en este host, así que no
    /// pueden divergir. Un campo `pub` permitiría a quien llama ensamblar
    /// Production sobre stores de `in_memory()`, que es el estado que este
    /// cambio existe para rechazar.
    profile: Profile,
}

impl EntityEventStores {
    pub fn profile(&self) -> Profile { self.profile }
}
```

**Por qué no IS-11 tal como está literalmente redactado.** IS-11 pide que
`build_runtime_with` declare `Profile::Production`. No puede: `build_runtime_with` no
es el punto de entrada exclusivo de producción que el proposal supone. Es el punto de
entrada *compartido*, invocado con stores in-memory desde cuatro lugares hoy —
`build_runtime_observed_in_memory` (`lib.rs:526-528`, al que
`build_runtime_in_memory` en `:311-315` delega),
`examples/reference-app/tests/stoolap_restart_persistence.rs:86` y `:134`, y
`examples/reference-app/tests/idempotency_wiring.rs:86`. Hardcodear Production dentro
rompe los cuatro y todo punto de entrada dev que la app tiene.

**Criterios**:

1. **Responde R-1 estructuralmente y no por convención.** R-1 — "hay que *recordar* la
   flag" — queda cerrado para este host no porque un chequeo vigile la declaración, sino
   porque no hay una declaración separada que olvidar: `EntityEventStores::open` es la
   única forma de obtener stores durables, y es lo único que produce Production.
   `main.rs:78` ya la invoca. Es una garantía estrictamente más fuerte que "una llamada
   de composición más un chequeo de regresión" de D-8, y no necesita ninguno de los dos.
2. **Cero churn en call sites.** `main.rs` (que invoca `open`) pasa a Production sin
   ninguna edición. Los cuatro llamadores in-memory de `build_runtime_with` quedan en
   Dev sin edición. Los cuatro tests de integración con Postgres que invocan
   `EntityEventStores::open` (`durable_entity_progress_postgres.rs:94`, `:390`, `:428`;
   `dual_aggregate_crash_recovery_postgres.rs:237`;
   `concurrent_replicas_postgres.rs:279`) pasan a Production sin edición y siguen
   válidos, porque `open` también provee los snapshot stores durables (AD-9).
3. **Coincide exactamente con el propósito existente del tipo.** `EntityEventStores` se
   introdujo para esta clase precisa de defecto; su propio doc comment dice que el
   default in-memory "significaba que cada evento y cada receipt vivía en memoria de
   proceso — y un reinicio perdía el progreso durable que los receipts existen para
   registrar" (`lib.rs:338-342`).

**Alternativa considerada — un parámetro `profile: Profile` en `build_runtime_with`.**
Más literal a IS-11, aproximadamente la misma cantidad de líneas. Rechazada porque la
guarda de regresión que permite es estrictamente más débil: `main.rs` es un binario, así
que ningún test puede invocarlo, y la única forma de probar que `main.rs` pasa
`Profile::Production` es grepear su texto fuente. AD-8 elimina la cosa que podría
regresionar en lugar de vigilarla.

**Nota de alcance**: `EntityEventStores` es azúcar local de la reference app, no API del
framework. `Profile` sigue siendo un método de primera clase de
`EntityRuntimeBuilder`/`AppBuilder` (AD-1, AD-5), así que un host con un store durable
que este tipo no conoce declara su profile directamente. AD-8 restringe un host de
ejemplo, no el framework.

**Consecuencia**: `observed_entity_runtime` (`lib.rs:488-510`) toma el snapshot store y
el profile, y devuelve `Result` porque ahora invoca `try_build()`.
`compose_entity_runtimes` (`lib.rs:452-471`) — público, e invocado con `in_memory()`
desde `tests/metrics_reach_one_backend.rs:209` — también se vuelve falible, o conserva
`build()`; con el profile privado y `open()` proveyendo siempre todos los stores, ningún
input construible puede hacerlo rechazar, así que conservar `build()` ahí es defendible
y más pequeño. Las tasks deben elegir uno y decir cuál; el requisito del diseño es solo
que, cualquiera se elija, la vía Dev siga siendo infalible para los llamadores
existentes.

### AD-9 — La vía de producción de la reference app gana `PostgreSQLSnapshotStore`, cerrando un defecto de durabilidad vivo que el gate expone

**Decisión**: `EntityEventStores::open(pool)` también construye dos instancias de
`PostgreSQLSnapshotStore` sobre el mismo pool; `EntityEventStores::in_memory()`
construye dos `InMemorySnapshotStore`.

```rust
// en open(pool), junto a las dos llamadas a PostgreSQLEventStore::open
org_snapshot: Arc::new(Mutex::new(PostgreSQLSnapshotStore::new(pool.clone()))),
user_snapshot: Arc::new(Mutex::new(PostgreSQLSnapshotStore::new(pool))),
```

**Por qué es requerido y no opcional**: sin esto, AD-8 hace que la composición de
producción de la reference app declare `Profile::Production` sin snapshot store
configurado, y el gate rechaza a su propio host de referencia. La alternativa — exceptuar
el snapshot store del gate — borraría IS-3 y SC-2.

**Por qué está en alcance**: OOS-1 excluye *construir* un backend durable.
`PostgreSQLSnapshotStore` ya existe (`crates/persistence/src/postgres/snapshot.rs:27`,
exportado en `crates/persistence/src/lib.rs:11`) y la tabla `snapshots` ya viene en las
migraciones. Nada se implementa acá; se cablean dos llamadas a constructor. La reference
app ya depende de `ego-persistence` para `PostgreSQLEventStore::open`.

**Qué revela esto**: el despliegue de producción de la reference app hoy escribe eventos
en Postgres y snapshots en memoria de proceso, silenciosamente. Es la clase exacta de
defecto que PROD-013 existe para cerrar, encontrada en el propio host de referencia del
cambio, y es la evidencia más fuerte disponible de que el gate hace trabajo real en
lugar de documentar cumplimiento.

**Dos instancias tipadas sobre un pool, no una compartida**: espeja el comentario
existente en `EntityEventStores` ("mismo pool, mismas tablas, mismas transacciones"). Un
`Arc<Mutex<...>>` compartido serializaría todo el I/O de snapshots de ambos agregados
detrás de un único lock, cosa que el default actual de `InMemorySnapshotStore` por
runtime no hace.

**Mina para la fase de tasks** — `PostgreSQLSnapshotStore::save_snapshot` invoca
`tokio::task::block_in_place` (`snapshot.rs:46-48`), que **panickea en un runtime
current-thread**. `main.rs` es `#[tokio::main]` (multi-thread por default), así que
producción está bien. Los tests de integración con Postgres usan `#[tokio::test]` pelado
(current-thread) — p. ej. `durable_entity_progress_postgres.rs:187`, `:233`, `:289`,
`:374`. Solo panickean si un save se dispara de verdad, y el default
`PeriodicSnapshotStrategy::new(100)` (`crates/persistent-entity/src/builder.rs:265`)
significa que menos de 100 eventos por agregado nunca dispara uno — que es la razón por
la que esto está latente hoy y no ya roto. Cualquiera de esos tests que pueda cruzar el
umbral debe pasar a `#[tokio::test(flavor = "multi_thread")]`. Registrado como riesgo, no
asumido resuelto.

### AD-10 — La guarda de regresión de IS-12 son dos aserciones de test, no un lint de `xtask`

**Decisión**: ningún subcomando nuevo de `xtask`. Dos aserciones de comportamiento:

1. `examples/reference-app/tests/production_profile_guard.rs` (nuevo) — asevera que
   `EntityEventStores::in_memory().profile() == Profile::Dev` y que `build_runtime_with`
   sobre stores in-memory sigue construyendo. Guarda la vía Dev y SC-5 en la raíz de
   composición.
2. Una aserción agregada al ya existente
   `integration-tests/tests/infrastructure/durable_entity_progress_postgres.rs` (que ya
   abre un pool real y ya invoca `EntityEventStores::open` en `:94` y luego
   `build_runtime_with` en `:112`) — `assert_eq!(stores.profile(), Profile::Production)`.
   Guarda la declaración de producción donde el pool que necesita ya existe.

**Criterios**:

1. **Un lint de `xtask` acá no lo correría nadie.** No hay CI: `.github/workflows/` no
   existe, y el `Makefile` no tiene target de `xtask` (grepeado por
   `xtask`/`verify-layers`/`verify-isolation`/`verify-hygiene` — cero matches). Los lints
   existentes se invocan a mano. Una guarda cuyo propósito entero es fallar un build que
   nunca la corre es una guarda solo de nombre, que es el mismo modo de falla que R-1 ya
   describe un nivel arriba.
2. **Los lints de `xtask` existentes existen porque su sujeto no tiene punto de
   observación en runtime.** `verify-layers` lee `cargo metadata` y `layers.toml`;
   `verify-isolation` compila cada crate con su propio feature set; `verify-hygiene`
   recorre `openspec/changes/`. Ninguno de esos hechos es observable desde un test. Una
   declaración de profile sí lo es: es un valor que una función devuelve.
3. **La superficie más chica que resuelve el problema** — el estándar declarado del
   proyecto. AD-10 es un archivo de test nuevo y chico más una línea en un test que ya
   existe. Un lint de `xtask` es un módulo nuevo, un brazo nuevo en `main.rs`, un string
   de uso nuevo, un parser de texto fuente para `lib.rs`, y un hook de `Makefile`/CI para
   que corra siquiera.
4. **Asevera comportamiento, no texto fuente.** Un lint textual que busque el literal
   `Profile::Production` en `lib.rs` pasa con una declaración dentro de código muerto, un
   comentario o un bloque `#[cfg(test)]`, y falla ante un refactor correcto que mueva el
   string. `stores.profile()` no puede satisfacerse con nada más que el valor real.

**Consecuencia, declarada y no maquillada**: la aserción 2 vive en una suite dependiente
de Docker, así que no se dispara en un `cargo test --workspace` pelado. Es aceptable
porque AD-8 ya eliminó el modo de falla que una guarda barata siempre-activa habría
cuidado — con el campo del profile privado y `open()` como su único productor de
Production, no hay forma de alcanzar una composición de producción sin pasar por el
código que la aserción 2 cubre. Si de todos modos se quiere una aserción sin Postgres, la
opción honesta es un constructor `#[cfg(test)]` en `EntityEventStores`, y eso es una
costura de testing en código de producción para compensar un chequeo que no tiene hueco
que cerrar — rechazado por el mismo criterio de "superficie más chica".

### AD-11 — Approach C: evaluado, dimensionado, diferido (D-7 / OOS-5)

Registrado solo por trazabilidad. **No implementado y no diseñado acá.**

**Qué es**: invertir el default para que `Profile::Production` (o un equivalente sin
nombre) sea lo que recibe un `EntityRuntimeBuilder::new()` pelado, con un único opt-out
nombrado — la forma exacta que `IdempotencyEnforcementMode::MandatoryKey` +
`Compatibility` ya tiene.

**Costo medido en `develop @ a740d34`**:

| Ítem | Cantidad | Evidencia |
|---|---|---|
| Call sites de `EntityRuntimeBuilder::new()` | 67, en 25 archivos | grepeado, R-2 saldado |
| …que no configuran ningún store | ~32, en 14 archivos | explore §2 |
| …que configuran exactamente uno | 15, en 8 archivos | EC-1 |
| **Sitios que necesitan el opt-out** | **~47, en ~20 archivos** | suma de las dos filas |
| Definiciones de helper estilo `compat()` necesarias | ~12 | ver abajo |

La cuenta de helpers es la parte que D-7 señala y la parte fácil de subestimar. El helper
propio de PROD-012 es `#[cfg(test)]`-local y de cuatro líneas
(`crates/service-sdk/src/runtime/builder.rs:1715-1718`), y PROD-012 ya necesitó una
**segunda** copia para la superficie de `AppBuilder` (`compat_app()`,
`crates/service-sdk/src/app/mod.rs:822-824`) porque un ítem `#[cfg(test)]` no puede
cruzar un módulo, mucho menos un crate o un binario de test de integración. Los sitios
afectados de PROD-013 abarcan los tests de `src` de `persistent-entity`, sus **8**
archivos separados bajo `crates/persistent-entity/tests/` (cada uno un binario
independiente que necesita su propia copia o un módulo `tests/support` compartido que hoy
no existe ahí), los tests de `src` de `service-sdk` y 2 archivos bajo su `tests/`,
`examples/reference-app/tests/`, e `integration-tests/tests/infrastructure/`. O ~12
definiciones duplicadas, o un export nuevo de `ego-testkit` más ~12 imports — y
`ego-testkit` es dev-only por política deliberada de layering
(`crates/persistent-entity/Cargo.toml:19-21`), así que esa ruta necesita su propia
aprobación.

**Total estimado**: ~47 ediciones de call site más ~12 definiciones de helper en ~20
archivos; aproximadamente 250–350 líneas de migración pura, tocando seis crates, con cero
ganancia de comportamiento sobre el Approach A una vez que AD-8 aterriza — porque AD-8
ya vuelve incapaz a la vía de producción del host de referencia de alcanzar
almacenamiento volátil por omisión.

**Por qué diferido y no rechazado**: sigue siendo el contrato de estado final más fuerte
para un host *de terceros* que nunca lee la reference app, al que AD-8 no protege (el
riesgo residual que R-1 acepta explícitamente). Si alguna vez se retoma, los números de
arriba son el inventario de partida, y debería ser su propio cambio con su propio plan de
migración — nunca doblado dentro de un cambio cuya propiedad aprobada es blast radius
cero.

---

## Puntos de Integración

| Límite | Dirección | Mecanismo | Verificado en |
|---|---|---|---|
| `persistent-entity` → `service-sdk` | arriba | `pub use persistent_entity::profile::Profile` | `crates/service-sdk/Cargo.toml:24`; estilo de reexport en `runtime/mod.rs:13-27` |
| `PersistenceCompositionError` → `RuntimeError` | arriba, solo effect store | `#[from]` | variante nueva; AD-5 |
| `RuntimeError` → `CompositionError` | arriba | `Validation(#[from] RuntimeError)`, ya presente | `app/error.rs:59`; mapeado en `app/mod.rs:807` |
| rechazo event/snapshot → host | afuera, sin cruce | `Result` de `try_build()`, absorbido con `?` | AD-6 |
| `AppBuilder::profile` → `RuntimeBuilder::profile` | abajo, fino | delegación, espejando `effect_store` | `app/mod.rs:562-578` |
| entity runtimes → `AppBuilder` | adentro, ya construidos | `with_entity(Arc<EntityRuntime<E>>)` — el gate de profile ya corrió | `runtime/builder.rs:3175-3177` |

## Estrategia de Testing

Según `ego-rs-testing-strategy`: unit tests donde vive la regla, tests de integración en
el límite del crate, end-to-end solo para la raíz de composición. TDD estricto — RED
antes de GREEN, cada vía de error aseverando el error específico y no `is_err()`.

| Nivel | Ubicación | Qué prueba |
|---|---|---|
| Unit | `crates/persistent-entity/src/profile.rs` `#[cfg(test)]` | `require_durably_configured` sobre la matriz completa: {Dev, Production} × {configurado con durabilidad, no} — cuatro casos, un test table-driven; más un test de regresión fijado probando que la presencia sola (`is_some()`) nunca se acepta como durabilidad |
| Unit | `crates/persistent-entity/src/builder.rs` `#[cfg(test)]` | SC-1, SC-2 (`try_build` rechaza, nombrando la capacidad **y** la llamada de arreglo), SC-5 (Dev + nada configurado igual construye), que `build()` panickea con el mismo input que `try_build()` rechaza, y que un store en memoria explícito bajo `Profile::Production` se rechaza exactamente igual que uno faltante |
| Unit | `crates/persistent-entity/src/error.rs` `#[cfg(test)]` | el mensaje nombra la capacidad y la llamada exacta de arreglo, espejando `the_refusal_names_the_registration_and_the_opt_out` de PROD-012 (IS-7) |
| Unit | `crates/service-sdk/src/runtime/builder.rs` `#[cfg(test)]` | SC-3; que sin executor registrado no hay rechazo (AD-5); que `build()`/`try_build()` coinciden |
| Integración | `crates/service-sdk/tests/` | el rechazo emerge como `CompositionError::Validation` a través de `AppBuilder::build()` |
| E2E | `examples/reference-app/tests/production_profile_guard.rs` | mitad Dev de SC-11, SC-5 en la raíz de composición (AD-10) |
| Integración (Docker) | `durable_entity_progress_postgres.rs` existente | mitad Production de SC-11 (AD-10) |

Dos propiedades negativas necesitan tests explícitos, porque son lo que SC-4 y SC-7
realmente aseveran y ninguna es demostrable por un happy path que pasa:

- **SC-4** — bajo `Profile::Production`, ninguna vía alcanza `InMemoryEventStore` ni
  `InMemorySnapshotStore`. Aseverar que el rechazo ocurre *antes* de la construcción, no
  que el runtime construido se ve bien; los brazos `unwrap_or_else` actuales
  (`builder.rs:279-286`) deben ser inalcanzables, y un test que inspecciona el runtime
  construido no puede distinguir "inalcanzable" de "alcanzado y luego sobrescrito".
- **SC-7** — `cargo test --workspace` con cero fallas nuevas en los 67 call sites. Es la
  totalidad de IS-8 y lo chequea la suite como conjunto, no un test nuevo.

## Trazabilidad

| Ítem del proposal | Resuelto por | Nota |
|---|---|---|
| IS-1, D-1 | AD-1 | |
| IS-2, IS-3, SC-1, SC-2 | AD-3, AD-4 | |
| IS-4, SC-3 | AD-5 | "en bootstrap y no en el primer uso" de SC-3 necesita reformularse por EC-2 |
| IS-5, SC-6, D-2 | **AD-7** | premisa falsificada (EC-1) — necesita enmienda del proposal |
| IS-6, SC-8 | AD-3 | un predicado, honesto a través del límite de capas |
| IS-7 | AD-2 | |
| IS-8, SC-7 | AD-4 | 67 sitios / 25 archivos reverificados; R-2 saldado |
| IS-9, IS-10, SC-9, SC-10 | solo documentación; sin decisión de diseño requerida | |
| IS-11, SC-11 | AD-8, AD-9 | redacción literal imposible (`build_runtime_with` es compartido) |
| IS-12 | AD-10 | test, no `xtask` |
| D-7, OOS-5, R-7 | AD-11 | dimensionado, diferido |
| R-3 | AD-4 | |
| R-4 | AD-2, AD-6 | más angosto de lo evaluado: un cruce, no dos |
| R-2 | saldado | 67/25 remedidos; EC-1 encontró el hueco que advertía |

## Ítems que Necesitan Confirmación del Arquitecto

1. **AD-7 / EC-1** — D-2, IS-5, SC-6 y el tercer bloque `Given` del criterio de
   aceptación necesitan enmienda. El chequeo incondicional de configuración parcial
   cuesta 15 migraciones de call site incluida la raíz de composición de producción de la
   reference app, no cero, y contradice IS-8/SC-7.
2. **EC-2** — "falla en el bootstrap en lugar del primer uso" de SC-3 describe un modo de
   falla que el effect store no tiene. El defecto real es el fallback silencioso a
   `InMemoryEffectStore` en `crates/service-sdk/src/runtime/builder.rs:811`. La redacción
   del spec debe seguir al código.
3. **AD-8 / AD-9** — IS-11 tal como está redactado no es implementable. Confirmar que el
   profile viaje en `EntityEventStores`, y confirmar que cablear el ya existente
   `PostgreSQLSnapshotStore` en la vía de producción de la reference app está en alcance
   (es cableado, no implementación, así que OOS-1 lo permite — pero es comportamiento
   nuevo en el host de referencia y expone un defecto de durabilidad vivo).

