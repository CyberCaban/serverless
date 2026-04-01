use thiserror::Error;

#[derive(Debug, Error)]
pub enum FunctionError {
    #[error("Функция не развернута")]
    FunctionNotDeployed,
    #[error("Нет запущенных контейнеров для функции")]
    NoRunningContainers,
}
