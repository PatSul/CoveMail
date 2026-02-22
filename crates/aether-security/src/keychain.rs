use crate::SecurityError;

#[derive(Debug, Clone)]
pub struct SecretStore {
    service_name: String,
    legacy_service_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SecretKey {
    pub namespace: String,
    pub id: String,
}

impl SecretKey {
    pub fn as_username(&self) -> String {
        format!("{}:{}", self.namespace, self.id)
    }
}

impl SecretStore {
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            legacy_service_name: None,
        }
    }

    pub fn new_with_legacy(
        service_name: impl Into<String>,
        legacy_service_name: impl Into<String>,
    ) -> Self {
        Self {
            service_name: service_name.into(),
            legacy_service_name: Some(legacy_service_name.into()),
        }
    }

    pub fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecurityError> {
        let entry = keyring::Entry::new(&self.service_name, &key.as_username())?;
        entry.set_password(value)?;
        Ok(())
    }

    pub fn get(&self, key: &SecretKey) -> Result<Option<String>, SecurityError> {
        let entry = keyring::Entry::new(&self.service_name, &key.as_username())?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => {
                let Some(legacy_service_name) = &self.legacy_service_name else {
                    return Ok(None);
                };
                let legacy_entry = keyring::Entry::new(legacy_service_name, &key.as_username())?;
                match legacy_entry.get_password() {
                    Ok(secret) => {
                        let _ = entry.set_password(&secret);
                        Ok(Some(secret))
                    }
                    Err(keyring::Error::NoEntry) => Ok(None),
                    Err(err) => Err(err.into()),
                }
            }
            Err(err) => Err(err.into()),
        }
    }

    pub fn delete(&self, key: &SecretKey) -> Result<(), SecurityError> {
        let entry = keyring::Entry::new(&self.service_name, &key.as_username())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(err) => return Err(err.into()),
        }

        if let Some(legacy_service_name) = &self.legacy_service_name {
            let legacy_entry = keyring::Entry::new(legacy_service_name, &key.as_username())?;
            match legacy_entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(err) => return Err(err.into()),
            }
        }

        Ok(())
    }

    pub fn service_name(&self) -> &str {
        &self.service_name
    }

    pub fn legacy_service_name(&self) -> Option<&str> {
        self.legacy_service_name.as_deref()
    }
}
