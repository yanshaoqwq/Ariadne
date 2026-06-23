use yaml_serde::{Mapping, Value};

use crate::core::{CoreError, CoreResult};

pub const CURRENT_CONFIG_SCHEMA_VERSION: u32 = 1;

/// 单个配置文件迁移结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    pub file_name: String,
    pub from_version: u32,
    pub to_version: u32,
    pub changed: bool,
}

/// 返回当前支持的配置 schema version。
pub fn current_schema_version() -> u32 {
    CURRENT_CONFIG_SCHEMA_VERSION
}

/// 从 YAML Value 中读取 schema_version，缺失时视为 0。
pub fn schema_version(value: &Value) -> u32 {
    value
        .as_mapping()
        .and_then(|mapping| mapping.get(Value::String("schema_version".to_owned())))
        .and_then(Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .unwrap_or(0)
}

/// 迁移单个 YAML 配置文件到当前 schema version。
pub fn migrate_yaml_value(file_name: &str, value: Value) -> CoreResult<(Value, MigrationReport)> {
    let from_version = schema_version(&value);
    if from_version > CURRENT_CONFIG_SCHEMA_VERSION {
        return Err(CoreError::validation(format!(
            "{file_name} schema version {from_version} is newer than supported version {CURRENT_CONFIG_SCHEMA_VERSION}"
        )));
    }

    let mut mapping = as_mapping(file_name, value)?;
    let changed = if from_version == CURRENT_CONFIG_SCHEMA_VERSION {
        false
    } else {
        // 当前版本只补 schema_version；后续破坏性迁移在这里追加步骤。
        mapping.insert(
            Value::String("schema_version".to_owned()),
            Value::Number(CURRENT_CONFIG_SCHEMA_VERSION.into()),
        );
        true
    };

    Ok((
        Value::Mapping(mapping),
        MigrationReport {
            file_name: file_name.to_owned(),
            from_version,
            to_version: CURRENT_CONFIG_SCHEMA_VERSION,
            changed,
        },
    ))
}

/// 配置文件必须是 YAML mapping；空文件按空 mapping 处理。
fn as_mapping(file_name: &str, value: Value) -> CoreResult<Mapping> {
    match value {
        Value::Mapping(mapping) => Ok(mapping),
        Value::Null => Ok(Mapping::new()),
        other => Err(CoreError::validation(format!(
            "{file_name} must contain a YAML mapping, got {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_adds_schema_version_once() {
        let value = Value::Mapping(Mapping::new());
        let (migrated, first) = migrate_yaml_value("app.yaml", value).unwrap();
        let (again, second) = migrate_yaml_value("app.yaml", migrated).unwrap();

        assert!(first.changed);
        assert!(!second.changed);
        assert_eq!(schema_version(&again), CURRENT_CONFIG_SCHEMA_VERSION);
    }
}
