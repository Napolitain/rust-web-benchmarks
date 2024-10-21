package main

import (
	"fmt"
	"net/http"
)

func helloHandler(w http.ResponseWriter, r *http.Request) {
	fmt.Fprintln(w, "Hello, World!")
}

func main() {
	http.HandleFunc("/", helloHandler)
	fmt.Println("Starting server on :8080")
	if err := http.ListenAndServe(":3000", nil); err != nil {
		fmt.Println("Server failed:", err)
	}
}
