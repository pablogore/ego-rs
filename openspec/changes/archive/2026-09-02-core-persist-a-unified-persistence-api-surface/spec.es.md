# Specs Delta: CORE-PERSIST-A — Superficie Unificada de API de Persistencia (Puertos Propiedad del Dominio)

> Documento acompañante para revisión. La fuente de verdad canónica es `spec.md` (identificadores
> de requisito y escenarios 1:1).
> Un solo archivo que cubre dos capabilities, según la sección de Capacidades de este cambio:
> una nueva (`persistence-api-surface`) y una modificada (`foundation-integrity`). Este cambio es
> puramente estructural — sin cambios de comportamiento en tiempo de ejecución (OOS-6) — por lo
> que cada escenario se formula en términos de compilación e identidad, no de comportamiento en
> tiempo de ejecución.

## Capability: `persistence-api-surface` (NUEVA)

### Propósito

El contrato observable de que el vocabulario de puertos de persistencia propiedad del
dominio — cada trait, tipo de error y tipo de contrato bajo `persistence/`, `read_side/` y
`operation/`, más el contrato `DomainEvent` y el generador de macro de identidad `id_type!` —
tiene exactamente un crate propietario (`ego-persistence-api`), y que cada ruta que un
consumidor resuelve hoy sigue resolviendo al mismo ítem después de este cambio. Esta capability
no cubre ninguna implementación (los adaptadores en memoria o de PostgreSQL permanecen donde
están) ni ningún cambio de firma, SQL o comportamiento.

## Requisitos

### Requisito: Cada Ítem Reubicado Se Mueve Textualmente

El sistema DEBE reubicar cada ítem público (trait, struct, enum, fn, type, const) declarado en
los módulos `persistence/`, `read_side/{offset,dedup,store,projection_state_store,event_tag,
state,event_stream}` y `operation/{reservation,key,receipt}` propiedad del dominio, más el
contrato `DomainEvent` (`event.rs`), hacia `ego-persistence-api`, sin editar salvo por la ruta
de módulo.

#### Escenario: Un trait se reubica sin editar
- DADO `OffsetStore` tal como se declara en `ego_domain::read_side::offset` antes de este cambio
- CUANDO se ubica en `ego_persistence_api::read_side::offset` después de este cambio
- ENTONCES su declaración es idéntica, salvo por la ruta de módulo que la contiene

#### Escenario: Una constante simple también se reubica, no solo traits y structs
- DADO que `operation::key::MAX_LEN` es una constante pública, no un trait ni un struct
- CUANDO se completa la reubicación
- ENTONCES resuelve en `ego_persistence_api::operation::key::MAX_LEN` exactamente igual que
  cualquier otro ítem reubicado

### Requisito: La Ruta Antigua Resuelve al Mismo Ítem

Cada ítem reubicado por este cambio DEBE seguir siendo resoluble, sin editar, en su ruta
`ego_domain::*` exacta previa al cambio, a través de un re-export a nivel de módulo, y DEBE
resolver al mismo ítem exacto — no a un ítem redeclarado que solo comparte el nombre.

#### Escenario: Un import sin editar sigue compilando
- DADO un crate que compila `use ego_domain::persistence::EventStore;` antes de este cambio
- CUANDO se compila después de este cambio con esa declaración sin editar
- ENTONCES compila, y el trait resuelto coacciona por identidad con
  `ego_persistence_api::persistence::event_store::EventStore`

#### Escenario: Un re-export existente a nivel de ítem dentro de ego-domain sigue resolviendo
- DADO que `ego_domain::persistence::mod.rs` republicaba `PersistenceError` a nivel de ítem
  antes de este cambio
- CUANDO el módulo contenedor se convierte en un re-export a nivel de módulo de
  `ego-persistence-api`
- ENTONCES `ego_domain::persistence::PersistenceError` sigue resolviendo, sin editar, al mismo
  tipo

### Requisito: La Forma del Trait Es Idéntica Byte a Byte

Las firmas de método, bounds, supertraits y cuerpos por defecto de cada trait reubicado DEBEN
ser idénticos antes y después de este cambio, difiriendo solo en la ruta de módulo.

#### Escenario: Un bound asíncrono sobrevive a la reubicación
- DADO `EventStore<E: DomainEvent>` tal como se declara antes de este cambio
- CUANDO se declara en `ego-persistence-api` después de este cambio
- ENTONCES su bound genérico, su conjunto de métodos y su forma `#[async_trait]` no cambian

### Requisito: Las Implementaciones de Reenvío `Arc<T>` Se Mueven Intactas

Las implementaciones de reenvío general `Arc<T>` de `OffsetStore` y `DedupStore` DEBEN
reubicarse junto con su trait y DEBEN seguir reenviando `is_durable()` al store interno.

#### Escenario: Un par durable sigue siendo durable detrás de Arc
- DADA una implementación de store cuyo `is_durable()` devuelve verdadero
- CUANDO se registra detrás de `Arc<dyn OffsetStore>` después de este cambio
- ENTONCES `is_durable()` sobre el `Arc` sigue devolviendo verdadero, no el valor por defecto
  del trait

### Requisito: La Macro `id_type!` Se Reubica y Se Reinvoca, No Se Duplica

La macro `id_type!` DEBE reubicarse a `ego-persistence-api` como `#[macro_export]`, generando
`TenantId`/`TenantIdError` allí. `ego-domain` DEBE generar sus otros cinco tipos de identidad
reinvocando la macro reubicada, no duplicando su definición.

#### Escenario: `TenantId` resuelve a través del generador reubicado
- DADO `TenantId` generado por la macro `id_type!` reubicada
- CUANDO se resuelve en `ego_domain::TenantId`
- ENTONCES es el tipo exacto que generó la macro reubicada, no una segunda definición

#### Escenario: Un tipo de identidad no reubicado sigue compilando desde un solo generador
- DADO que `AggregateId` se genera en `ego-domain` y no se reubica él mismo
- CUANDO `ego-domain` compila después de este cambio
- ENTONCES `AggregateId` se genera invocando la macro re-exportada, y solo existe una
  definición de `id_type!` en todo el workspace

### Requisito: Ningún Consumidor Fuera de los Dos Crates Es Editado

Ningún crate distinto de `ego-domain` y `ego-persistence-api` DEBE tener una declaración `use`
editada ni una nueva dependencia en `Cargo.toml` como resultado de este cambio.

#### Escenario: Un consumidor downstream compila sin editar
- DADO un crate que importa un ítem reubicado solo a través de `ego_domain::*`
- CUANDO el workspace se reconstruye después de este cambio
- ENTONCES el código fuente y el `Cargo.toml` de ese crate son idénticos byte a byte a antes
  del cambio

### Requisito: `ego-persistence-api` No Depende de Ningún Crate del Workspace

`ego-persistence-api` NO DEBE declarar una dependencia `path` hacia ningún otro crate del
workspace, incluyendo `ego-domain`.

#### Escenario: El nuevo crate compila de forma aislada
- DADO `crates/persistence-api/Cargo.toml`
- CUANDO se inspecciona
- ENTONCES no nombra ninguna dependencia `path` del workspace, y el crate compila de forma
  independiente

### Requisito: Los Ítems Conocidos Como Muertos Se Reubican Sin Nuevo Comportamiento

`ProjectionStateStore` DEBE reubicarse tal cual, con cero implementaciones y cero consumidores,
y NO DEBE ganar una implementación ni ser eliminado por este cambio.

#### Escenario: Un trait muerto sigue muerto después de la reubicación
- DADO que `ProjectionStateStore` tiene cero implementaciones antes de este cambio
- CUANDO se busca en el workspace implementaciones después de este cambio
- ENTONCES sigue teniendo cero implementaciones, y sigue resolviendo en su ruta antigua

## No-Objetivos

- Ningún cambio de SQL, migración, índice, transacción, reintento o pool de conexiones de
  ningún tipo.
- Ningún cambio de firma de método, async/sync, `Send`/`Sync`, ni de object-safety en ningún
  trait reubicado.
- Ninguna reubicación de implementación — cada adaptador `InMemory*` y `PostgreSQL*`/`Postgres*`
  permanece en su crate actual.
- Ninguna reubicación de los puertos de effect-store propiedad de `ego-runtime`
  (`EffectStateStore`, `EffectDedupStore`, `RetentionMaintenance`) — diferida a un seguimiento
  (F-1).
- Ninguna corrección del defecto confirmado de scoping de tenant/`ON CONFLICT` de
  `PostgreSQLRepository` — se registra como deuda nombrada (F-2).
- Ningún arnés de conformidad agregado para `Repository`, `Snapshot`, `OffsetStore` ni
  `DedupStore`.

## Capability: `foundation-integrity` (MODIFICADA)

## Requisitos MODIFICADOS

### Requisito: FR-002 — Aplicación de la Dirección de Dependencias

Las dependencias de un crate NO DEBEN violar la dirección de capas documentada:
transport/infrastructure PUEDE depender solo de domain y application; application PUEDE
depender solo de domain; domain NO DEBE depender de ninguna otra capa, EXCEPTO que un crate de
la capa domain PUEDE depender de otro crate de la capa domain (el auto-borde de domain).

(Anteriormente: domain NO DEBÍA depender de ninguna otra capa, sin permitir ningún auto-borde —
un crate `domain` no podía depender de nada, ni siquiera de otro crate `domain`.)

#### Escenario: Una dependencia en dirección incorrecta hace fallar la puerta
- DADO que un crate de una capa depende de un crate en una capa que las reglas de dirección
  prohíben
- CUANDO se ejecuta la puerta
- ENTONCES falla, identificando el crate infractor y la regla de dirección violada

#### Escenario: Un auto-borde domain-a-domain pasa la puerta
- DADO que un crate de la capa domain depende de otro crate de la capa domain
- CUANDO se ejecuta la puerta
- ENTONCES pasa para ese borde, y no se reporta ninguna violación para él

#### Escenario: Domain sigue sin poder depender de foundation o infrastructure
- DADO que un crate de la capa domain depende de un crate de la capa foundation o
  infrastructure
- CUANDO se ejecuta la puerta
- ENTONCES sigue fallando, identificando el crate infractor y la regla de dirección violada
