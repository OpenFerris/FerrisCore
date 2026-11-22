# FerrisCore

Base mínima de Ferris: 9 crates sin verticales ni host wasm.

- `ferris-core`, `ferris-macro`, `ferris-db`, `ferris-schema`,
  `ferris-document`, `ferris-api`, `ferris-auth`, `ferris-web-shared`,
  `ferris-web-ui`.

El producto completo (Desk, verticales, wasm, servidor y CLI) vive en
`FerrisFramework`, que depende de este repositorio por rutas
`../../FerrisCore/ferris-*`. Los dos repositorios deben estar hermanos.

```bash
cargo check --workspace --all-targets
cargo test --workspace
```
