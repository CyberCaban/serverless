Тема: «Проектирование и реализация бессерверной (serverless) архитектуры(громко сказано)».
Как это работает:

1. Вы пишете функции на Python/Node.js/Go/Rust
2. Платформа автоматически создает Docker-контейнеры для каждой функции
3. При первом запросе - контейнер "просыпается" (cold start)
4. При последующих - работает быстро (warm start)

Цели:
· Реализовать работающий прототип
· Протестировать производительность
· Исследовать проблему холодных стартов
· Сравнить с традиционной архитектурой

Темы подробнее:
1. Cold Start Problem - как уменьшить задержку при первом запуске функции?
   · Предварительный прогрев функций
   · Оптимизация размеров контейнеров
   · Использование более легковесных runtime
2. Управление состоянием - функции stateless, как работать с данными?
   · Оптимизация работы с внешними БД
   · Кэширование метаданных
3. Мониторинг и отладка - как отслеживать выполнение распределенных функций?

TODO:
- Добавить автоскейлинг + scale-to-zero.
- Восстановление состояния после рестарта процесса (сейчас часть состояния в памяти).


Example workflow:
1. Create directory with your serverless function
2. add Dockerfile
3. add json schema in functions/your-fn/function.json:
```json
{
  "name": "your-fn",
  "innerPort": 3000,
  "memory": 128,
  "timeout": 30,
  "replicas": 1,
  "loadBalancer": "round_robin",
  "replicaWeights": [1],
  "version": "1.0.0",
  "dockerfile": "./path-to-dockerfile",
  "entrypoint": "hello-world"
}
```
4. Deploy function:
```bash
curl -X POST http://localhost:5000/deploy/your-fn
```
Deploying will create docker image
5. Invoke function:
```bash
curl -X POST http://localhost:5000/invoke/your-fn \
  -H "Content-Type: application/json" \
  -d '{ "name": "yourName", "priority": 5 }'
```
Invoking will run your function with passed parameters and return result in JSON

REQUEST/RESPONSE schema:

Platform → Function: HTTP POST /invoke
Headers:
    X-Request-Id: uuid
    X-Function-Name: hello-world
Body: request in JSON

Function → Platform: HTTP Response
Status: 200 OK
Body: response in JSON

Pre-requisites:
- tar
- docker

Links 
https://github.com/fnproject/fn - open source, Functions-as-a-Service (FaaS) compute platform.


Benchmark script (Rust)

Скрипт для нагрузочного теста находится в `src/bin/invoke_bench.rs`.

Перед запуском:
1. Поднимите платформу (порт 5000).
2. Разверните тестируемую функцию (`example-go`, `example-js` или `example-rust`).

Пример запуска:
```bash
cargo run --bin invoke_bench -- \
  --base-url http://localhost:5000 \
  --function example-go \
  --duration-secs 30 \
  --concurrency 20 \
  --timeout-ms 2500 \
  --output bench_report.json
```

Параметры:
- `--base-url` - адрес API платформы
- `--function` - имя функции для теста
- `--duration-secs` - длительность теста
- `--concurrency` - количество параллельных воркеров
- `--timeout-ms` - таймаут на один запрос
- `--output` - путь к JSON-файлу отчета
- `--pattern` - режим нагрузки: `stress`, `stable`, `pulse`
- `--target-rps` - целевая интенсивность для `stable`
- `--low-rps` - низкий уровень нагрузки для `pulse`
- `--high-rps` - высокий уровень нагрузки для `pulse`
- `--pulse-period-secs` - длина одного пульса для `pulse`

Режимы нагрузки:
1. `stress` - запросы идут максимально быстро, ограничение задаёт только система.
2. `stable` - постоянная интенсивность, примерно `$\lambda(t) \approx const$`.
3. `pulse` - чередование высокого и низкого уровня нагрузки по периодам.

Примеры:
```bash
cargo run --release --bin invoke_bench -- \
  --function example-go \
  --pattern stable \
  --target-rps 500 \
  --output bench_stable.json
```

```bash
cargo run --release --bin invoke_bench -- \
  --function example-go \
  --pattern pulse \
  --low-rps 100 \
  --high-rps 1200 \
  --pulse-period-secs 5 \
  --output bench_pulse.json
```

```bash
cargo run --release --bin invoke_bench -- \
  --function example-go \
  --pattern stress \
  --output bench_stress.json
```

В итоговый JSON попадают ключевые метрики:
1. Средняя задержка, p95 и p99.
2. Доля ошибок и таймаутов.
3. Равномерность загрузки контейнеров (hits per container, max deviation, Jain fairness index).
4. Итоговая пропускная способность (RPS).

Benchmark matrix runner (Rust)

Скрипт для матричного прогона конфигураций находится в `src/bin/function_matrix.rs`.

Поддерживаемые параметры:
- `--replicas` - список реплик через запятую, например `1,2`
- `--bench-pattern` - один из `stress`, `stable`, `pulse`, либо `all`
- `--all-bench-patterns` - запустить сразу `stable`, `pulse`, `stress`
- `--all-three-cases` - один прогон для трёх кейсов: `stress`, `stable(300000 rps)`, `pulse(100000..300000, 5s)`
- `--target-rps` - целевой RPS для `stable`
- `--low-rps` - нижняя точка для `pulse`
- `--high-rps` - пиковая точка для `pulse`
- `--pulse-period-secs` - период пульса для `pulse`

В `matrix summary` для каждого прогона добавляется `expected_rps`:
- `stable`: `target_rps`
- `pulse`: среднее `(low_rps + high_rps) / 2`
- `stress`: `null` (не фиксируется заранее)

Команды для быстрых прогонов только на репликах 1 и 2:

```bash
cargo run --release --bin function_matrix -- \
  --function example-go \
  --replicas 1,2 \
  --all-three-cases \
  --output-dir ./results
```

```bash
cargo run --release --bin function_matrix -- \
  --function example-go \
  --replicas 1,2 \
  --bench-pattern stress \
  --output-dir ./results
```

```bash
cargo run --release --bin function_matrix -- \
  --function example-go \
  --replicas 1,2 \
  --bench-pattern stable \
  --target-rps 300000 \
  --output-dir ./results
```

```bash
cargo run --release --bin function_matrix -- \
  --function example-go \
  --replicas 1,2 \
  --bench-pattern pulse \
  --low-rps 100000 \
  --high-rps 300000 \
  --pulse-period-secs 5 \
  --output-dir ./results
```

ДЛЯ example-js
1. Макс нагрузка 3300 rps
2. Для одного макс нагрузка 2500 rps
2. пульсирующая нагрузка должна в пике быть >3300 rps