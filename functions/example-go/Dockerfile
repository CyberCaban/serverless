FROM golang:1.22-alpine AS builder
RUN apk add --no-cache ca-certificates
WORKDIR /app
COPY go.mod ./
COPY main.go ./
RUN go build -o app main.go

FROM alpine:3.20
RUN apk add --no-cache ca-certificates curl
WORKDIR /app
COPY --from=builder /app/app ./app
EXPOSE 3000
CMD ["./app"]
