# Conformance corpus

Small hand-written fixtures that pin the subset boundary, committed so the
reader is tested without network access.

- structural.sysml - part def, attribute def, features, specialization.
- requirement.sysml - requirement, subject, satisfy.
- out-of-scope.sysml - calc def, state def, actor, usecase, association: every
  one must be a named Unmappable loss, never a panic.
- malformed.sysml - an unterminated doc comment: a clean typed import error.

The REAL conformance corpus is the bundled example set at
sample/examples/sysml-v2/ (the GfSE community models, the MBSE4U Batmobile and
the Airbus Apollo 11 mission). The real-example tests point at the two smallest
models there - Drone_BaseArchitecture.sysml and InternetModel_v1.sysml - by
path; the large models (Apollo 11, the mining frigate, the Batmobile) are
referenced, not copied, because they are already committed in the repository.
