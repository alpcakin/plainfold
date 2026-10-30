// Class-name helper adapter. Handles: the packages that only join or choose the class names an element is given
// (`clsx`, `classnames`, `tailwind-merge`, `class-variance-authority`, `tailwind-variants`). A call into them says
// nothing about what the program does, so it is not drawn as a call into a package: a component that does nothing
// else is no story. Nothing else is known about these packages.
use super::Adapter;

pub struct Styling;

const PACKAGES: &[&str] = &["clsx", "classnames", "tailwind-merge", "class-variance-authority", "tailwind-variants"];

impl Adapter for Styling {
    fn name(&self) -> &'static str {
        "class-name helpers"
    }
    fn langs(&self) -> &'static [&'static str] {
        &["typescript"]
    }
    fn handles(&self, package: &str) -> bool {
        PACKAGES.contains(&package)
    }
}
