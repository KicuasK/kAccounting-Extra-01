# ADR 0049: Successful export download capability

Accepted technical design, 2026-10-22.

Honor the user's Downloads-first workflow using a single binary transfer container. Successful export production records the exact verified ciphertext object descriptors in a private process-local registry under the opaque operation ID. Status metadata alone does not confer download authority.

Download revalidates directory identity and all descriptors; transport writes additionally validate every streamed object. Fixed attachment filenames and no-store headers prevent header injection and discourage persistence outside the explicit download. Bounded worker/channel streaming avoids loading complete packages or plaintext. Interrupted output is incomplete and must fail strict container extraction.

Only encrypted objects and the public recipient envelope are downloadable. This transport format does not replace recipient authentication or import validation. The binary upload endpoint feeds the full authenticated container import helper while retaining the shared storage permit. Browser picker/upload UI and Safari acceptance are deferred to their authorized implementation stage.
