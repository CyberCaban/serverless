package main

import (
    "encoding/json"
    "fmt"
    "net/http"
)

type sortRequest struct {
    Numbers []int `json:"numbers"`
}

type sortResponse struct {
    Sorted []int `json:"sorted"`
}

type errorResponse struct {
    Error string `json:"error"`
}

func quickSort(nums []int) []int {
    if len(nums) <= 1 {
        return nums
    }

    pivot := nums[len(nums)/2]

    var less, equal, greater []int
    for _, n := range nums {
        switch {
        case n < pivot:
            less = append(less, n)
        case n == pivot:
            equal = append(equal, n)
        default:
            greater = append(greater, n)
        }
    }

    result := make([]int, 0, len(nums))
    result = append(result, quickSort(less)...)
    result = append(result, equal...)
    result = append(result, quickSort(greater)...)
    return result
}

func main() {
    handleQuickSort := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
        if r.Method != http.MethodPost {
            http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
            return
        }

        defer r.Body.Close()

        var req sortRequest
        if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
            w.Header().Set("Content-Type", "application/json")
            w.WriteHeader(http.StatusBadRequest)
            _ = json.NewEncoder(w).Encode(errorResponse{Error: "Invalid JSON"})
            return
        }

        sorted := quickSort(req.Numbers)
        w.Header().Set("Content-Type", "application/json")
        _ = json.NewEncoder(w).Encode(sortResponse{Sorted: sorted})
    })
    handleOk := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
        fmt.Fprintln(w, "OK")
    })
    handleNotFound := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
        http.NotFound(w, r)
    })

    http.Handle("/", handleQuickSort)
    http.Handle("/ok", handleOk)
    http.Handle("/notfound", handleNotFound)
    if err := http.ListenAndServe(":8080", nil); err != nil {
        fmt.Printf("Server error: %v\n", err)
    }
}