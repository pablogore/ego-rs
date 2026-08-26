# Delta para Application Composition

## Requisitos AGREGADOS

### Requisito: Declaración de Profile en la Raíz de Composición

`RuntimeBuilder` y `AppBuilder` DEBEN aceptar una declaración de `Profile`
(`Profile::Dev` por defecto, re-exportado hacia arriba desde
`persistent-entity`, `Profile::Production`). Este es el mismo tipo
`Profile` que condiciona a `EntityRuntimeBuilder`, no un segundo concepto
paralelo.

#### Escenario: AppBuilder sin profile declarado usa Dev por defecto
- DADO una composición `AppBuilder` que nunca declara un profile
- CUANDO se construye
- ENTONCES se comporta como `Profile::Dev`, idéntico al comportamiento
  actual

### Requisito: Puerta del Effect Store Bajo Production, Condicionada a un Executor Registrado, Expuesta a través de CompositionError

Bajo `Profile::Production`, cuando hay al menos un effect executor
registrado, la composición DEBE rechazar `build()` cuando no se configuró
explícitamente un effect store vía `RuntimeBuilder::with_effect_store()` o
`AppBuilder::effect_store()`, nombrando la capacidad faltante y la llamada
de configuración de la superficie en uso. Este rechazo DEBE reutilizar la
plantilla de validador/error de PROD-012 (`validate_idempotency()`,
`crates/service-sdk/src/runtime/builder.rs:735-771`) y DEBE exponerse a
través del camino existente
`CompositionError::Validation(#[from] RuntimeError)` — no se introduce
ningún mecanismo nuevo de reporte de errores. El rechazo DEBE ocurrir en
el arranque: el effect store tiene un fallback silencioso real hacia
`InMemoryEffectStore` (`crates/service-sdk/src/runtime/builder.rs:811`),
la misma volatilidad en el arranque que cierran las puertas de event y
snapshot store — no una falla diferida al primer uso. Cuando no hay ningún
effect executor registrado, no se construye ningún effect store, así que
no hay nada volátil que custodiar.

#### Escenario: Effect store faltante bajo Production rechaza en el build cuando hay un executor registrado
- DADO `Profile::Production`, al menos un effect executor registrado, y
  ninguna llamada a `.with_effect_store()` / `.effect_store()`
- CUANDO corre `AppBuilder::build()` / `RuntimeBuilder::build()`
- ENTONCES rechaza a través de `CompositionError::Validation`, nombrando el
  effect store y la llamada de configuración exacta — el mismo fallback en
  el arranque que cierran las puertas de event y snapshot store, nunca
  surgiendo más tarde en el primer intento de uso

#### Escenario: Ningún effect executor registrado significa nada que custodiar bajo Production
- DADO `Profile::Production` y ningún effect executor registrado
- CUANDO la composición construye
- ENTONCES tiene éxito sin importar si se configuró un effect store — no se
  construye ningún effect store, así que nada volátil es alcanzable

#### Escenario: El profile Dev sin effect store mantiene el fallback silencioso actual, sin cambios
- DADO `Profile::Dev` (el default) y ningún effect store configurado
- CUANDO la composición construye
- ENTONCES tiene éxito, cayendo silenciosamente a `InMemoryEffectStore`
  exactamente como hoy — no una falla diferida al primer uso, ni un
  rechazo

### Requisito: La Reference App Propaga su Profile Desde EntityEventStores, Custodiado por un Chequeo de Regresión

`build_runtime_with` (`lib.rs:567`) de `examples/reference-app` DEBE
declarar su profile en la cadena `AppBuilder`/`RuntimeBuilder` que compone
(`App::builder()...`) propagando el profile que ya carga el valor
`EntityEventStores` que recibió — vía el accesor `.profile()` de ese
valor — en lugar de un literal hardcodeado, porque `build_runtime_with` es
el punto de entrada compartido que se llama tanto con
`EntityEventStores::open()` (Production) como con
`EntityEventStores::in_memory()` (Dev); hardcodear `Profile::Production`
adentro rompería cada llamador en memoria. `build_runtime_in_memory`
(`lib.rs:311`) y `build_runtime_observed_in_memory` (`lib.rs:522`) DEBEN
seguir alcanzando `Profile::Dev` a través de
`EntityEventStores::in_memory()`, sin ninguna declaración separada. Un
chequeo (un lint de `xtask` o un test — el mecanismo exacto es una decisión
de `design.md`) DEBE fallar el build si la composición alcanzada a través
de `EntityEventStores::open()` (`main.rs`) alguna vez deja de resultar en
una composición `AppBuilder`/`RuntimeBuilder` con `Profile::Production`.

#### Escenario: build_runtime_with propaga Production cuando recibe stores durables
- DADO `build_runtime_with` llamado con `EntityEventStores::open(pool)`
- CUANDO se inspecciona el profile de la composición
  `AppBuilder`/`RuntimeBuilder` resultante
- ENTONCES es `Profile::Production`, y la configuración de
  event/snapshot/effect stores durables satisface esa puerta

#### Escenario: build_runtime_with propaga Dev cuando recibe stores en memoria
- DADO `build_runtime_with` llamado con `EntityEventStores::in_memory()`
- CUANDO se inspecciona el profile de la composición
  `AppBuilder`/`RuntimeBuilder` resultante
- ENTONCES es `Profile::Dev`, sin cambios respecto de hoy

#### Escenario: Remover el cableado de producción hace fallar el chequeo de regresión
- DADO la composición alcanzada a través de `EntityEventStores::open()`
  (`main.rs`) que ya no resulta en una composición `Profile::Production`
- CUANDO corre el chequeo de regresión
- ENTONCES falla, nombrando la declaración faltante
