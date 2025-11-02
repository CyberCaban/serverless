-- Add migration script here
CREATE TABLE IF NOT EXISTS functions (
    id TEXT PRIMARY KEY,           -- UUID функции
    name TEXT NOT NULL,            -- Имя функции
    status TEXT NOT NULL,          -- pending, building, ready, failed, deleted
    source_code TEXT,              -- Исходный код или путь к нему
    runtime TEXT NOT NULL,         -- python, nodejs, go и т.д.
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Индексы для таблицы functions
CREATE INDEX IF NOT EXISTS idx_functions_status ON functions (status);
CREATE INDEX IF NOT EXISTS idx_functions_created_at ON functions (created_at);

-- Таблица сборок (builds)
CREATE TABLE IF NOT EXISTS builds (
    id TEXT PRIMARY KEY,           -- UUID сборки
    function_id TEXT NOT NULL,     -- Ссылка на функцию
    status TEXT NOT NULL,          -- pending, building, success, failed
    docker_image TEXT,             -- Имя собранного образа
    build_logs TEXT,               -- Логи сборки (можно вынести в отдельную таблицу для больших логов)
    started_at DATETIME,
    completed_at DATETIME,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (function_id) REFERENCES functions (id) ON DELETE CASCADE
);

-- Индексы для таблицы builds
CREATE INDEX IF NOT EXISTS idx_builds_function_id ON builds (function_id);
CREATE INDEX IF NOT EXISTS idx_builds_status ON builds (status);

-- Таблица вызовов функций
CREATE TABLE IF NOT EXISTS invocations (
    id TEXT PRIMARY KEY,           -- UUID вызова
    function_id TEXT NOT NULL,     -- Ссылка на функцию
    status TEXT NOT NULL,          -- running, success, failed, timeout
    input_data TEXT,               -- Входные данные (или ссылка на них)
    output_data TEXT,              -- Выходные данные
    error_message TEXT,            -- Сообщение об ошибке
    started_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    completed_at DATETIME,
    execution_time_ms INTEGER,     -- Время выполнения в миллисекундах

    FOREIGN KEY (function_id) REFERENCES functions (id) ON DELETE CASCADE
);

-- Индексы для таблицы invocations
CREATE INDEX IF NOT EXISTS idx_invocations_function_id ON invocations (function_id);
CREATE INDEX IF NOT EXISTS idx_invocations_status ON invocations (status);
CREATE INDEX IF NOT EXISTS idx_invocations_started_at ON invocations (started_at);

-- Таблица для хранения логов выполнения
CREATE TABLE IF NOT EXISTS execution_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invocation_id TEXT NOT NULL,
    log_level TEXT NOT NULL,       -- INFO, ERROR, DEBUG
    message TEXT NOT NULL,
    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (invocation_id) REFERENCES invocations (id) ON DELETE CASCADE
);

-- Индексы для таблицы execution_logs
CREATE INDEX IF NOT EXISTS idx_execution_logs_invocation_id ON execution_logs (invocation_id);
CREATE INDEX IF NOT EXISTS idx_execution_logs_timestamp ON execution_logs (timestamp);
