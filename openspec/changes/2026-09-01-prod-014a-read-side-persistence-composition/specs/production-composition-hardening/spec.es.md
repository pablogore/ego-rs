# Delta para production-composition-hardening

> Documento acompañante para revisión. La fuente de verdad canónica es
> `spec.md` (identificadores 1:1). Spec base de la capacidad: el propio
> delta de PROD-013,
> `openspec/changes/2026-08-25-prod-013-production-composition-hardening/specs/production-composition-hardening/spec.md`
> (aún no archivado en `openspec/specs/`; este delta se aplica sobre él y
> ambos se aplican juntos al momento del archivado).

Alcance: PROD-014A. Extiende la puerta de Production para incluir el
progreso durable del read-side como una cuarta capacidad gobernada (event
store, snapshot store, effect store, progreso durable del read-side),
reutilizando el mismo mecanismo (`is_durable()` +
`require_durably_configured`) y la misma forma de error
(`PersistenceCompositionError::NotConfigured { capability, fix }`,
expuesto a través de `CompositionError::Validation`) que estableció
PROD-013.

## Requisitos AGREGADOS

### Requisito: Puerta de Progreso Durable del Read-Side Bajo Production, Aplicada en el Bootstrap, Condicionada a una Proyección Registrada

Bajo `Profile::Production`, cuando al menos el par de progreso durable de
una proyección (`OffsetStore` + `DedupStore`) está registrado en la raíz
de composición, `AppBuilder::build()` DEBE rechazar el bootstrap cuando
cualquiera de los dos stores de ese par no sea durable, nombrando la
capacidad faltante o no durable y la llamada de registro exacta que la
resuelve. Esto refleja la condicionalidad de la puerta del effect store:
cuando no hay ninguna proyección registrada, no existe ningún par de
progreso durable del read-side que construir, así que no hay nada
volátil que rechazar — una aplicación solo de comandos o sin read-side
nunca se ve forzada a registrar un store ficticio. La puerta recorre el
mismo camino existente que estableció PROD-013
(`AppBuilder::build()` -> `RuntimeBuilder::try_build()` ->
`validate_persistence_profile()` -> `RuntimeError` ->
`CompositionError::Validation`), nunca un segundo validador paralelo. La
durabilidad se determina exclusivamente por `is_durable()` en cada store,
alimentando `require_durably_configured()` — nunca por `.is_some()` ni
por ninguna otra heurística.

#### Escenario: Un store volátil en un par registrado es rechazado en el bootstrap

- DADO `Profile::Production` y una proyección registrada cuyo
  `OffsetStore` o `DedupStore` no es durable
- CUANDO se ejecuta `AppBuilder::build()`
- ENTONCES se rechaza, nombrando la capacidad faltante o no durable y la
  llamada de registro exacta que la resuelve — nunca diferido a
  `ProjectionSpec::new()`, `TagSchedulerImpl::spawn()`, ni el primer lote

#### Escenario: Ninguna proyección registrada implica que no hay nada que rechazar

- DADO `Profile::Production` y ningún progreso de proyección de read-side
  registrado
- CUANDO se ejecuta `AppBuilder::build()`
- ENTONCES tiene éxito — no se construye ningún par de progreso durable
  del read-side, así que no hay nada volátil alcanzable

#### Escenario: Ambos stores durables tiene éxito

- DADO `Profile::Production` y una proyección registrada cuyo
  `OffsetStore` y `DedupStore` son ambos durables
- CUANDO se ejecuta `AppBuilder::build()`
- ENTONCES tiene éxito

#### Escenario: El profile Dev con stores volátiles no cambia

- DADO `Profile::Dev` (por defecto) y una proyección registrada usando
  `OffsetStore`/`DedupStore` en memoria/volátiles
- CUANDO se ejecuta `AppBuilder::build()`
- ENTONCES tiene éxito, byte por byte como antes de este cambio

### Requisito: El Comentario de Documentación de Profile::Production Refleja el Slot de Progreso Durable del Read-Side

El comentario de documentación de `Profile::Production`
(`crates/persistent-entity/src/profile.rs`) NO DEBE afirmar que la
persistencia de read-side/proyección "has no such slot yet and is
deliberately not governed here" una vez que este cambio se despliegue —
esa afirmación se vuelve falsa en el momento en que el registro existe.
El comentario DEBE en cambio nombrar el par de progreso durable del
read-side como una cuarta capacidad gobernada junto al event store, el
snapshot store y el effect store, y NO DEBE apuntar a un identificador
sucesor como trabajo aún pendiente para una capacidad que este cambio ya
gobierna.

#### Escenario: El comentario de documentación lista la cuarta capacidad gobernada

- DADO el comentario de documentación de `Profile::Production` después de
  que este cambio se despliega
- CUANDO se lee
- ENTONCES lista el par de progreso durable del read-side junto al event
  store, el snapshot store y el effect store como capacidades
  gobernadas, y no afirma que el read-side carece de un slot en la raíz
  de composición

## Requisitos MODIFICADOS

### Requisito: Un Único Predicado Compartido Es la Única Fuente de Verdad de la Regla

Exactamente un predicado compartido DEBE decidir "production declarado +
capacidad no configurada de forma durable = rechazar" para las cuatro
capacidades (event store, snapshot store, effect store, progreso durable
del read-side). Debido a que las capacidades viven a través de un límite
de crate unidireccional (`persistent-entity` no puede ver los tipos de
effect-store ni de read-side de `service-sdk`), este predicado no puede
inspeccionar ninguno de los builders directamente: cada superficie de
composición (`validate_persistence()` de `EntityRuntimeBuilder`,
`validate_persistence_profile()` de `RuntimeBuilder`, incluyendo su rama
de read-side) DEBE calcular localmente su propia respuesta para su
capacidad y pasarla al único predicado compartido — nunca reafirmar la
decisión de rechazar/permitir por su cuenta. NO DEBE existir una segunda
definición, mantenida de forma independiente, de la decisión en ningún
punto del camino de composición.

(Anteriormente: alcanzaba tres capacidades, event store/snapshot
store/effect store; ahora incluye el progreso durable del read-side como
una cuarta, validada por el mismo `validate_persistence_profile()` que ya
usa el effect store.)

#### Escenario: La decisión de las tres capacidades originales pasa por el mismo predicado

- DADO el camino de composición desde `EntityRuntimeBuilder` y
  `RuntimeBuilder`/`AppBuilder`
- CUANDO se inspecciona el código base en busca de lógica de puerta de
  capacidad para event store, snapshot store o effect store
- ENTONCES cada punto de la puerta calcula su propia respuesta local y la
  pasa al único predicado compartido que decide rechazar-o-permitir;
  ningún punto reimplementa esa decisión por su cuenta

#### Escenario: La decisión de la cuarta capacidad pasa por el mismo predicado

- DADO la puerta de progreso durable del read-side agregada por este
  cambio
- CUANDO se inspecciona el código base en busca de su lógica de puerta
- ENTONCES calcula su propia respuesta local (¿son durables ambos stores
  de un par registrado?) y la pasa al mismo predicado compartido que ya
  usan las otras tres capacidades — no existe una decisión separada,
  mantenida de forma independiente, exclusiva del read-side

### Requisito: Los Rechazos Son Accionables

Todo rechazo bajo este spec DEBE nombrar tanto la capacidad faltante como
la llamada de configuración exacta que la resuelve.

(Anteriormente: enumeraba tres capacidades en su escenario; ahora incluye
el progreso durable del read-side como una cuarta.)

#### Escenario: El error nombra la capacidad y la solución

- DADO cualquier rechazo producido por la puerta de este spec
- CUANDO se inspecciona el error
- ENTONCES nombra la capacidad faltante (event store, snapshot store,
  effect store, o progreso durable del read-side) y la llamada de
  registro o de builder exacta que la configura
