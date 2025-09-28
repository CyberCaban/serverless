Тема: «Проектирование и реализация бессерверной (serverless) архитектуры(громко сказано)».
Как это работает:

1. Вы пишете функции на Python/Node.js/Go
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


Example workflow:
1. Create directory with your serverless function
2. add Dockerfile
3. add json schema in functions/your-fn/function.json:
```json
{
  "name": "your-fn",
  "memory": 128,
  "timeout": 30,
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
  -d '{ "name": "yourName" }'
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



