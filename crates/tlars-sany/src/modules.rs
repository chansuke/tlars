//! `SpecObj.loadSpec` + `ParseUnit`: parse the root, then repeatedly resolve the next
//! unresolved EXTENDS, then INSTANCE (`findNextUnresolvedExtention` /
//! `findNextUnresolvedInstantiation`), printing `Parsing file <abs>[ (<uri>)]` per file;
//! circularity check; `calculateDependencies` for semantic-analysis order. The parse order
//! here is part of the intern sequence.

use crate::{Interner, ModuleResolver, SanyFailed, SanyOut};
use std::path::Path;

pub(crate) struct ParsedUnits {
    // ParseUnits in semanticAnalysisVector order
}

pub(crate) fn load_spec(
    root: &Path,
    resolver: &dyn ModuleResolver,
    interner: &Interner,
    out: &mut dyn SanyOut,
) -> Result<ParsedUnits, SanyFailed> {
    todo!()
}
