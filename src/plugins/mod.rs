pub mod php;
pub mod pgsql;
pub mod yii2;

pub fn registry() -> crate::analyzer::PluginRegistry {
    crate::analyzer::PluginRegistry::new(
        vec![
            Box::new(php::PhpLanguagePlugin),
            Box::new(pgsql::PgsqlLanguagePlugin),
        ],
        vec![Box::new(yii2::Yii2Plugin)],
    )
    .expect("built-in plugin IDs are unique")
}
