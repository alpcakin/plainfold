// React Native adapter. Handles: the `react-native` package is the platform a React Native app runs on, as the
// browser's own objects are for a web page, so its calls (dialogs, the keyboard, animation, layout) are not drawn
// as calls into a package; and what it draws runs on a device.
use super::Adapter;
use crate::model::Group;

pub struct ReactNative;

impl Adapter for ReactNative {
    fn name(&self) -> &'static str {
        "React Native"
    }
    fn langs(&self) -> &'static [&'static str] {
        &["typescript"]
    }
    fn handles(&self, package: &str) -> bool {
        package == "react-native"
    }
    fn draws_on(&self) -> Option<Group> {
        Some(Group::Device)
    }
}
