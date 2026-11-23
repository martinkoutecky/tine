(ns edn-reader-oracle
  (:require [clojure.edn :as edn]
            ["node:fs" :as fs]
            ["node:buffer" :refer [Buffer]]))

(defn code-units [text]
  (mapv #(.charCodeAt text %) (range (.-length text))))

(defn number-bits [value]
  (if (js/isNaN value)
    "7ff8000000000000"
    (let [buffer (Buffer/alloc 8)]
      (.writeDoubleBE buffer value)
      (.toString buffer "hex"))))

(defn items [array]
  (mapv #(aget array %) (range (.-length array))))

(defn observe [value]
  (cond
    (nil? value) ["nil"]
    (boolean? value) ["bool" value]
    (number? value) ["number" (number-bits value)]
    (string? value) ["string" (code-units value)]
    (keyword? value) ["keyword" (subs (str value) 1)]
    (symbol? value) ["symbol" (str value)]
    (vector? value) ["vector" (mapv observe value)]
    (set? value) ["set" (mapv observe value)]
    (map? value) ["map" (mapv (fn [[k v]] [(observe k) (observe v)]) value)]
    (list? value) ["list" (mapv observe value)]
    (sequential? value) ["queue" (mapv observe value)]
    (uuid? value) ["uuid" (code-units (str value))]
    (inst? value) ["instant" (.getTime value)]
    (array? value) ["js-vector" (mapv observe (items value))]
    :else ["js-map" (mapv (fn [k] [(code-units k) (observe (aget value k))])
                          (items (.keys js/Object value)))]))

(defn read-case [[operation source]]
  ;; An enclosing vector asks tools.reader to consume each form in the stream.
  ;; A synthetic final newline terminates any final line comment.
  (let [wrapped? (not= operation "observe-first")
        end :tine.oracle/end-of-input
        [verdict value] (try
                          [:ok (edn/read-string (if wrapped? (str "[" source "\n " end "]") source))]
                          (catch :default _ [:refused nil]))]
    (when (and (= verdict :ok) (or (not wrapped?) (= end (peek value))))
      (case operation
        "observe-stream" ["stream" (mapv observe (pop value))]
        "observe-strict" (when (= 2 (count value)) (observe (first value)))
        (observe value)))))

(let [cases (js->clj (.parse js/JSON (fs/readFileSync 0 "utf8")))]
  (println (.stringify js/JSON (clj->js (mapv read-case cases)))))
