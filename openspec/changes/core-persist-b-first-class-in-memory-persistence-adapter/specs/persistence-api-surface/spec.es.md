# Delta de Spec: CORE-PERSIST-B — Re-Alcance de las Declaraciones Propias de CORE-PERSIST-A en `persistence-api-surface`

> Documento acompañante para revisión. La fuente de verdad canónica es `spec.md` (identificadores
> de requisito y escenarios 1:1).
> Este delta toca exactamente dos declaraciones de la spec publicada `persistence-api-surface`:
> un Requisito y una viñeta de No-Objetivos. Ambas se escribieron para describir el límite propio
> de CORE-PERSIST-A, pero están formuladas como absolutos permanentes y atemporales. Leídas tal
> cual, harían que las ediciones legítimas y separadamente delimitadas de consumidores de
> CORE-PERSIST-B, así como su reubicación de implementación, parecieran violaciones de esta
> capability. Ningún otro requisito, escenario o viñeta de No-Objetivos en
> `openspec/specs/persistence-api-surface/spec.md` es tocado por este delta. Ningún requisito
> sobre la forma de los puertos, la resolución de rutas o la identidad de traits cambia.

## Capability: `persistence-api-surface` (MODIFICADA)

## Requisitos MODIFICADOS

### Requisito: Ningún Consumidor Fuera de los Dos Crates Es Editado Por CORE-PERSIST-A

Este requisito vincula específicamente a CORE-PERSIST-A: ningún crate distinto de `ego-domain` y
`ego-persistence-api` DEBE haber tenido una declaración `use` editada ni una dependencia añadida
en `Cargo.toml` como resultado de CORE-PERSIST-A. Restringe el diff histórico propio de
CORE-PERSIST-A y NO DEBE leerse como una prohibición permanente que vincule a todo cambio futuro
sobre esta capability. Un cambio posterior, propuesto de forma independiente, PUEDE editar
consumidores fuera de estos dos crates para su propia reubicación y estrategia de compatibilidad
explícitamente declaradas, siempre que ese cambio declare su propio alcance.

(Anteriormente: formulado como una regla atemporal sin calificar — "Ningún crate distinto de
`ego-domain` y `ego-persistence-api` DEBE tener una declaración `use` editada ni una nueva
dependencia en `Cargo.toml` como resultado de este cambio" — sin ningún anclaje textual que
identificara "este cambio" específicamente como CORE-PERSIST-A, lo que permitía que las ediciones
legítimas de consumidores de un cambio posterior se leyeran como una violación de este requisito.)

#### Escenario: Un consumidor downstream compila sin editar bajo CORE-PERSIST-A

- DADO un crate que importa un ítem reubicado solo a través de `ego_domain::*`
- CUANDO el workspace se reconstruye después de CORE-PERSIST-A
- ENTONCES el código fuente y el `Cargo.toml` de ese crate son idénticos byte a byte a antes de
  CORE-PERSIST-A

#### Escenario: Un cambio posterior puede editar consumidores dentro de su propio alcance declarado

- DADO un cambio propuesto de forma independiente y con alcance propio (por ejemplo
  CORE-PERSIST-B) que declara su propio alcance de edición de consumidores y su estrategia de
  compatibilidad
- CUANDO ese cambio edita declaraciones `use` en crates distintos de `ego-domain` y
  `ego-persistence-api` (por ejemplo `examples/reference-app` o `ego-testkit`)
- ENTONCES este requisito no se viola, porque vincula el diff propio de CORE-PERSIST-A, no todo
  cambio futuro sobre esta capability

## No-Objetivos MODIFICADOS

- Ninguna reubicación de implementación estaba en el alcance de CORE-PERSIST-A — cada adaptador
  `InMemory*` y `PostgreSQL*`/`Postgres*` permaneció en su crate actual a partir de CORE-PERSIST-A.
  Un cambio posterior, con alcance propio y revisado por separado (por ejemplo CORE-PERSIST-B),
  PUEDE reubicar una implementación; esta viñeta no lo vincula.

  (Anteriormente: formulado como una declaración permanente sin calificar — "Ninguna reubicación
  de implementación — cada adaptador `InMemory*` y `PostgreSQL*`/`Postgres*` permanece en su
  crate actual" — sin ningún anclaje a CORE-PERSIST-A, lo que se leía como una prohibición
  permanente de reubicar jamás una implementación, en lugar de una declaración de lo que
  CORE-PERSIST-A mismo dejó sin tocar.)
