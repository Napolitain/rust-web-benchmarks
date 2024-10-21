package main

import (
	"fmt"

	"github.com/valyala/fasthttp"
)

func helloHandler(ctx *fasthttp.RequestCtx) {
	ctx.WriteString("Hello, World!")
}

func main() {
	fmt.Println("Starting server on :8080")

	// Using fasthttp server
	if err := fasthttp.ListenAndServe(":3000", helloHandler); err != nil {
		fmt.Println("Server failed:", err)
	}
}
